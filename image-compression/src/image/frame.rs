use crate::error::Error;
use crate::ffi;
use crate::image::options::ImageFormat;
use crate::image::{decode, encode, resize};

/// A decoded image frame: the safe hand-off point between decoding,
/// transformation and encoding.
///
/// Frames are produced by [`Frame::decode`] and can be re-encoded with
/// [`Frame::encode`]. The underlying FFmpeg `AVFrame` is available to
/// same-workspace crates (e.g. `image-transformation`) through
/// [`Frame::as_raw_frame`] so filter graphs can run on it directly —
/// the pointer stays valid for the lifetime of this value.
///
/// # Example
///
/// ```no_run
/// use image_compression::{Frame, ImageFormat};
///
/// # let bytes: Vec<u8> = Vec::new();
/// let frame = Frame::decode(&bytes)?;
/// println!("{}x{} alpha={}", frame.width(), frame.height(), frame.has_alpha());
/// let jpeg = frame.encode(ImageFormat::Jpeg, 85)?;
/// # Ok::<(), image_compression::Error>(())
/// ```
pub struct Frame {
    inner: ffi::wrappers::Frame,
    source_format: Option<ImageFormat>,
}

impl Frame {
    pub(crate) fn new(inner: ffi::wrappers::Frame, source_format: Option<ImageFormat>) -> Frame {
        Frame {
            inner,
            source_format,
        }
    }

    /// Decodes an encoded image (any format FFmpeg can probe from
    /// content) into a frame.
    ///
    /// AVIF input is rejected with [`Error::UnsupportedFormat`] (FFmpeg
    /// ships no AVIF demuxer).
    pub fn decode(input: &[u8]) -> Result<Frame, Error> {
        decode::decode(input)
    }

    /// Frame width in pixels.
    pub fn width(&self) -> u32 {
        self.inner.width()
    }

    /// Frame height in pixels.
    pub fn height(&self) -> u32 {
        self.inner.height()
    }

    /// Whether the decoded pixels carry an alpha channel.
    pub fn has_alpha(&self) -> bool {
        self.inner.has_alpha()
    }

    /// Container format the input was probed as, when it maps to a
    /// supported [`ImageFormat`] (`Jpeg`, `Png`, `WebP`).
    ///
    /// `None` for inputs probed as anything else (GIF, BMP, TIFF, …) —
    /// callers that need a default should pick a lossless fallback.
    pub fn source_format(&self) -> Option<ImageFormat> {
        self.source_format
    }

    /// Encodes the frame, converting pixel format and rounding
    /// dimensions as required by the target format (JPEG/AVIF are
    /// rounded up to even values).
    pub fn encode(&self, format: ImageFormat, quality: u8) -> Result<Vec<u8>, Error> {
        let spec = encode::spec_for(format);
        let (width, height) = resize::round_to_even(self.width(), self.height(), spec.force_even);
        encode::encode_prepared(&self.inner, format, quality, width, height)
    }

    /// Raw `AVFrame *` for direct FFmpeg interop (filter graphs).
    ///
    /// # Safety contract
    ///
    /// The pointer is valid only while this [`Frame`] is alive; the
    /// caller must not free it or outlive the owning value.
    pub fn as_raw_frame(&self) -> *mut std::ffi::c_void {
        self.inner.as_ptr().cast()
    }

    /// Takes ownership of a raw `AVFrame *` produced by FFmpeg (e.g. a
    /// filter-graph sink frame) and wraps it as a [`Frame`].
    ///
    /// # Safety
    ///
    /// - `raw` must be a valid pointer from `av_frame_alloc` /
    ///   `av_buffersink_get_frame` (or equivalent).
    /// - Ownership is transferred: the pointer must not be freed or
    ///   reused by the caller afterwards.
    /// - `raw` must not be aliased by any other owner for the lifetime
    ///   of the returned [`Frame`].
    pub unsafe fn from_raw(
        raw: *mut std::ffi::c_void,
        source_format: Option<ImageFormat>,
    ) -> Result<Frame, Error> {
        if raw.is_null() {
            return Err(Error::InvalidInput);
        }
        Ok(Frame {
            inner: ffi::wrappers::Frame::from_ptr(raw.cast()),
            source_format,
        })
    }

    pub(crate) fn raw(&self) -> &ffi::wrappers::Frame {
        &self.inner
    }
}
