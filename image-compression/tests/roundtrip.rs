use image_compression::{CompressionOptions, ImageCompressor, ImageFormat};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .expect("fixture missing")
}

fn compress(
    input: &[u8],
    format: ImageFormat,
    quality: u8,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<Vec<u8>, image_compression::Error> {
    let compressor = ImageCompressor::new()?;
    compressor.compress(
        input,
        CompressionOptions {
            format,
            quality,
            width,
            height,
        },
    )
}

fn is_jpeg(data: &[u8]) -> bool {
    data.len() > 2 && data[0] == 0xFF && data[1] == 0xD8
}

fn is_png(data: &[u8]) -> bool {
    data.len() > 8 && &data[..8] == b"\x89PNG\r\n\x1a\n"
}

fn is_webp(data: &[u8]) -> bool {
    data.len() > 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP"
}

fn is_avif(data: &[u8]) -> bool {
    data.len() >= 12 && &data[4..8] == b"ftyp" && &data[8..12] == b"avif"
}

fn png_dimensions(data: &[u8]) -> (u32, u32) {
    assert!(is_png(data), "not a png");
    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    (width, height)
}

#[test]
fn png_input_to_all_formats() {
    let input = fixture("input.png");
    for (format, check) in [
        (ImageFormat::Jpeg, is_jpeg as fn(&[u8]) -> bool),
        (ImageFormat::Png, is_png),
        (ImageFormat::WebP, is_webp),
        (ImageFormat::Avif, is_avif),
    ] {
        let output = compress(&input, format, 80, None, None).unwrap();
        assert!(check(&output), "invalid output for {format:?}");
    }
}

#[test]
fn all_input_formats_to_jpeg() {
    for name in ["input.png", "input.jpg", "input.webp"] {
        let input = fixture(name);
        let output = compress(&input, ImageFormat::Jpeg, 80, None, None).unwrap();
        assert!(is_jpeg(&output), "invalid jpeg from {name}");
    }
}

#[test]
fn alpha_png_encodes_to_all_formats() {
    let input = fixture("input_alpha.png");
    for (format, check) in [
        (ImageFormat::Jpeg, is_jpeg as fn(&[u8]) -> bool),
        (ImageFormat::Png, is_png),
        (ImageFormat::WebP, is_webp),
        (ImageFormat::Avif, is_avif),
    ] {
        let output = compress(&input, format, 80, None, None).unwrap();
        assert!(check(&output), "invalid output for {format:?}");
    }
}

#[test]
fn resize_preserves_dimensions_and_aspect() {
    let input = fixture("input.png");

    let output = compress(&input, ImageFormat::Png, 80, Some(64), None).unwrap();
    assert_eq!(png_dimensions(&output), (64, 48));

    let output = compress(&input, ImageFormat::Png, 80, None, Some(60)).unwrap();
    assert_eq!(png_dimensions(&output), (80, 60));

    let output = compress(&input, ImageFormat::Png, 80, Some(100), Some(50)).unwrap();
    assert_eq!(png_dimensions(&output), (100, 50));

    let output = compress(&input, ImageFormat::Png, 80, None, None).unwrap();
    assert_eq!(png_dimensions(&output), (160, 120));
}

#[test]
fn jpeg_quality_controls_output_size() {
    let input = fixture("input.png");
    let low = compress(&input, ImageFormat::Jpeg, 10, None, None).unwrap();
    let high = compress(&input, ImageFormat::Jpeg, 95, None, None).unwrap();
    assert!(low.len() < high.len());
}

#[test]
fn avif_input_is_unsupported() {
    let input = fixture("input.avif");
    let err = compress(&input, ImageFormat::Png, 80, None, None).unwrap_err();
    assert_eq!(err, image_compression::Error::UnsupportedFormat);
}

#[test]
fn garbage_input_is_invalid() {
    let err = compress(b"not an image", ImageFormat::Jpeg, 80, None, None).unwrap_err();
    assert_eq!(err, image_compression::Error::InvalidInput);
}

#[test]
fn empty_input_is_invalid() {
    let err = compress(b"", ImageFormat::Jpeg, 80, None, None).unwrap_err();
    assert_eq!(err, image_compression::Error::InvalidInput);
}

#[test]
fn zero_dimensions_are_invalid() {
    let input = fixture("input.png");
    let err = compress(&input, ImageFormat::Jpeg, 80, Some(0), None).unwrap_err();
    assert_eq!(err, image_compression::Error::InvalidInput);
}

#[test]
fn out_of_range_quality_is_clamped() {
    let input = fixture("input.png");
    let output = compress(&input, ImageFormat::WebP, 255, None, None).unwrap();
    assert!(is_webp(&output));
}

#[test]
fn jpeg_output_for_odd_dimensions() {
    let input = fixture("input.png");
    let output = compress(&input, ImageFormat::Jpeg, 80, Some(63), Some(47)).unwrap();
    assert!(is_jpeg(&output));
}
