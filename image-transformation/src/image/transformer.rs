use super::filters;
use super::options::TransformOptions;
use crate::ffi::{FilterGraph, avfilter_get_by_name};
use image_conversion::{Error, Frame, ImageFormat};

/// FFmpeg-backed image transformer.
///
/// Stateless: each [`transform`](ImageTransformer::transform) call
/// decodes, builds one filter graph, runs a single graph pass and
/// encodes — no intermediate Rust pixel buffers.
///
/// # Example
///
/// ```no_run
/// use image_transformation::{ImageTransformer, TransformOperation, TransformOptions};
/// use image_conversion::ImageFormat;
///
/// # let input_bytes: Vec<u8> = Vec::new();
/// let transformer = ImageTransformer::new()?;
/// let output = transformer.transform(
///     &input_bytes,
///     TransformOptions {
///         operations: vec![
///             TransformOperation::Crop { x: 0, y: 0, width: 800, height: 600 },
///             TransformOperation::Rotate { degrees: 90 },
///             TransformOperation::Grayscale,
///         ],
///         custom_filters: None,
///         format: Some(ImageFormat::WebP),
///         quality: Some(80),
///     },
/// )?;
/// # Ok::<(), image_conversion::Error>(())
/// ```
pub struct ImageTransformer;

impl ImageTransformer {
    /// Verifies that libavfilter exposes the `buffer` and `buffersink`
    /// filters required by the pipeline.
    pub fn new() -> Result<ImageTransformer, Error> {
        unsafe {
            if avfilter_get_by_name(c"buffer".as_ptr()).is_null()
                || avfilter_get_by_name(c"buffersink".as_ptr()).is_null()
            {
                return Err(Error::FfmpegError(
                    "libavfilter buffer/buffersink filters unavailable".to_string(),
                ));
            }
        }
        Ok(ImageTransformer)
    }

    /// Transforms one encoded image in a single filter-graph pass.
    ///
    /// Pipeline: `Frame::decode` → `buffer → chain → buffersink` →
    /// `Frame::encode` (which also rounds odd dimensions up to even
    /// for JPEG/AVIF).
    ///
    /// The output format is `options.format`, else the input format,
    /// else [`ImageFormat::Png`] when the input format is unknown.
    pub fn transform(&self, input: &[u8], options: TransformOptions) -> Result<Vec<u8>, Error> {
        let decoded = Frame::decode(input)?;
        let source_format = decoded.source_format();

        let chain = filters::build_chain(
            decoded.width(),
            decoded.height(),
            decoded.has_alpha(),
            &options.operations,
            options.custom_filters.as_deref(),
        )?;

        let raw = decoded.as_raw_frame().cast();
        let graph = unsafe { FilterGraph::build(raw, &chain) }?;
        unsafe { graph.push(raw)? };
        let sink = graph.pull()?;
        let transformed = unsafe { Frame::from_raw(sink.into_raw().cast(), source_format) }?;

        let format = options.format.or(source_format).unwrap_or(ImageFormat::Png);
        let quality = options.quality.unwrap_or(80);
        transformed.encode(format, quality)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TransformOperation;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .expect("fixture missing")
    }

    /// `(r, g, b)` at `(x, y)` for the layouts a PNG decode can
    /// yield; `None` if the layout is not understood.
    fn pixel_at(frame: &Frame, x: u32, y: u32) -> Option<(u8, u8, u8)> {
        let raw = frame.as_raw_frame().cast::<crate::ffi::AVFrame>();
        unsafe {
            let fmt = (*raw).format as crate::ffi::AVPixelFormat;
            let plane = (*raw).data;
            let lines = (*raw).linesize;
            let (sx, sy) = (x as usize, y as usize);
            Some(match fmt {
                crate::ffi::AV_PIX_FMT_RGB24 => {
                    let p = plane[0].add(sy * lines[0] as usize + sx * 3);
                    (p.read(), p.add(1).read(), p.add(2).read())
                }
                crate::ffi::AV_PIX_FMT_BGR24 => {
                    let p = plane[0].add(sy * lines[0] as usize + sx * 3);
                    (p.add(2).read(), p.add(1).read(), p.read())
                }
                crate::ffi::AV_PIX_FMT_RGBA => {
                    let p = plane[0].add(sy * lines[0] as usize + sx * 4);
                    (p.read(), p.add(1).read(), p.add(2).read())
                }
                crate::ffi::AV_PIX_FMT_GBRP | crate::ffi::AV_PIX_FMT_GBRAP => (
                    plane[2].add(sy * lines[2] as usize + sx).read(),
                    plane[0].add(sy * lines[0] as usize + sx).read(),
                    plane[1].add(sy * lines[1] as usize + sx).read(),
                ),
                _ => return None,
            })
        }
    }

    #[test]
    fn grayscale_makes_every_pixel_neutral() {
        let input = fixture("input.png");

        // The fixture must actually contain color for this to mean
        // anything.
        let decoded = Frame::decode(&input).unwrap();
        let mut colorful = None;
        for y in 0..decoded.height() {
            for x in 0..decoded.width() {
                let (r, g, b) = pixel_at(&decoded, x, y).expect("unexpected input layout");
                if r != g || g != b {
                    colorful = Some((x, y, (r, g, b)));
                    break;
                }
            }
            if colorful.is_some() {
                break;
            }
        }
        let (x, y, before) = colorful.expect("fixture has no colorful pixel");

        let output = ImageTransformer::new()
            .unwrap()
            .transform(
                &input,
                TransformOptions {
                    operations: vec![TransformOperation::Grayscale],
                    ..Default::default()
                },
            )
            .unwrap();

        let frame = Frame::decode(&output).unwrap();
        let first = pixel_at(&frame, 0, 0).expect("unexpected output layout");
        assert_eq!(first.0, first.1, "pixel (0,0) not gray: {first:?}");
        assert_eq!(first.1, first.2, "pixel (0,0) not gray: {first:?}");

        let at = pixel_at(&frame, x, y).expect("unexpected output layout");
        assert_eq!(at.0, at.1, "pixel ({x},{y}) not gray: {at:?}");
        assert_eq!(at.1, at.2, "pixel ({x},{y}) not gray: {at:?}");
        assert_ne!(at, before, "grayscale changed nothing at ({x},{y})");
    }
}
