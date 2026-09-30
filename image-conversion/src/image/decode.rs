use crate::error::Error;
use crate::ffi::AvioReader;
use crate::ffi::Demuxer;
use crate::ffi::wrappers::Frame as AvFrame;
use crate::image::frame::Frame;
use crate::image::options::ImageFormat;

fn is_avif(data: &[u8]) -> bool {
    if data.len() < 12 || &data[4..8] != b"ftyp" {
        return false;
    }
    matches!(&data[8..12], b"avif" | b"avis" | b"mif1" | b"miaf")
}

fn format_from_probe(name: &str) -> Option<ImageFormat> {
    if name.contains("jpeg") || name.contains("jpg") {
        Some(ImageFormat::Jpeg)
    } else if name.contains("png") {
        Some(ImageFormat::Png)
    } else if name.contains("webp") {
        Some(ImageFormat::WebP)
    } else {
        None
    }
}

pub(crate) fn decode(input: &[u8]) -> Result<Frame, Error> {
    if input.is_empty() {
        return Err(Error::InvalidInput);
    }
    if is_avif(input) {
        return Err(Error::UnsupportedFormat);
    }

    let reader = AvioReader::new(input)?;
    let mut demux = Demuxer::open(reader)?;
    let source_format = demux.format_name().and_then(format_from_probe);
    let inner: AvFrame = demux.read_video_frame()?;
    Ok(Frame::new(inner, source_format))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_names_map_to_formats() {
        assert_eq!(format_from_probe("jpeg_pipe"), Some(ImageFormat::Jpeg));
        assert_eq!(format_from_probe("image2"), None);
        assert_eq!(format_from_probe("png_pipe"), Some(ImageFormat::Png));
        assert_eq!(format_from_probe("apng_pipe"), Some(ImageFormat::Png));
        assert_eq!(format_from_probe("webp_pipe"), Some(ImageFormat::WebP));
        assert_eq!(format_from_probe("gif_pipe"), None);
        assert_eq!(format_from_probe("bmp_pipe"), None);
        assert_eq!(format_from_probe("tiff_pipe"), None);
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn source_format_detected_from_fixtures() {
        assert_eq!(
            Frame::decode(&fixture("input.jpg"))
                .unwrap()
                .source_format(),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(
            Frame::decode(&fixture("input.png"))
                .unwrap()
                .source_format(),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            Frame::decode(&fixture("input.webp"))
                .unwrap()
                .source_format(),
            Some(ImageFormat::WebP)
        );
    }
}
