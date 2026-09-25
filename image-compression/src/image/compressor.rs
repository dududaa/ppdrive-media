use crate::error::Error;
use crate::ffi;
use crate::image::options::CompressionOptions;
use crate::image::{decode, encode, resize};

pub struct ImageCompressor;

impl ImageCompressor {
    pub fn new() -> Result<Self, Error> {
        unsafe {
            if ffi::av_log_get_level() > ffi::AV_LOG_ERROR as i32 {
                ffi::av_log_set_level(ffi::AV_LOG_ERROR as i32);
            }
        }
        for id in [
            ffi::AV_CODEC_ID_MJPEG,
            ffi::AV_CODEC_ID_PNG,
            ffi::AV_CODEC_ID_WEBP,
            ffi::AV_CODEC_ID_AV1,
        ] {
            ffi::wrappers::find_encoder_by_id(id)?;
        }
        Ok(ImageCompressor)
    }

    pub fn compress(&self, input: &[u8], options: CompressionOptions) -> Result<Vec<u8>, Error> {
        if options.width == Some(0) || options.height == Some(0) {
            return Err(Error::InvalidInput);
        }

        let frame = decode::decode(input)?;
        let spec = encode::spec_for(options.format);

        let (width, height) = resize::target_dimensions(frame.width(), frame.height(), &options)?;
        let (width, height) = resize::round_to_even(width, height, spec.force_even);

        let dst_fmt = spec.pix_fmt(frame.has_alpha());
        let frame = if frame.width() == width
            && frame.height() == height
            && frame.format() == dst_fmt as i32
        {
            frame
        } else {
            resize::convert(frame, width, height, dst_fmt)?
        };

        encode::encode(&frame, &options, &spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::options::ImageFormat;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn webp_roundtrip_preserves_alpha() {
        let input = fixture("input_alpha.png");
        let compressor = ImageCompressor::new().unwrap();
        let output = compressor
            .compress(
                &input,
                CompressionOptions {
                    format: ImageFormat::WebP,
                    quality: 80,
                    width: None,
                    height: None,
                },
            )
            .unwrap();
        let frame = decode::decode(&output).unwrap();
        assert!(frame.has_alpha());
        assert_eq!(frame.width(), 64);
        assert_eq!(frame.height(), 64);
    }

    #[test]
    fn png_roundtrip_preserves_alpha() {
        let input = fixture("input_alpha.png");
        let compressor = ImageCompressor::new().unwrap();
        let output = compressor
            .compress(
                &input,
                CompressionOptions {
                    format: ImageFormat::Png,
                    quality: 80,
                    width: None,
                    height: None,
                },
            )
            .unwrap();
        let frame = decode::decode(&output).unwrap();
        assert!(frame.has_alpha());
    }
}
