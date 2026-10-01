use ppff_image_conversion::{ConversionOptions, ImageConverter, ImageFormat};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .expect("fixture missing")
}

fn convert(
    input: &[u8],
    format: ImageFormat,
    quality: u8,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<Vec<u8>, ppff_image_conversion::Error> {
    let converter = ImageConverter::new()?;
    converter.convert(
        input,
        ConversionOptions {
            format,
            quality,
            width,
            height,
            ..Default::default()
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
        let output = convert(&input, format, 80, None, None).unwrap();
        assert!(check(&output), "invalid output for {format:?}");
    }
}

#[test]
fn all_input_formats_to_jpeg() {
    for name in ["input.png", "input.jpg", "input.webp"] {
        let input = fixture(name);
        let output = convert(&input, ImageFormat::Jpeg, 80, None, None).unwrap();
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
        let output = convert(&input, format, 80, None, None).unwrap();
        assert!(check(&output), "invalid output for {format:?}");
    }
}

#[test]
fn resize_preserves_dimensions_and_aspect() {
    let input = fixture("input.png");

    let output = convert(&input, ImageFormat::Png, 80, Some(64), None).unwrap();
    assert_eq!(png_dimensions(&output), (64, 48));

    let output = convert(&input, ImageFormat::Png, 80, None, Some(60)).unwrap();
    assert_eq!(png_dimensions(&output), (80, 60));

    let output = convert(&input, ImageFormat::Png, 80, Some(100), Some(50)).unwrap();
    assert_eq!(png_dimensions(&output), (100, 50));

    let output = convert(&input, ImageFormat::Png, 80, None, None).unwrap();
    assert_eq!(png_dimensions(&output), (160, 120));
}

#[test]
fn jpeg_quality_controls_output_size() {
    let input = fixture("input.png");
    let low = convert(&input, ImageFormat::Jpeg, 10, None, None).unwrap();
    let high = convert(&input, ImageFormat::Jpeg, 95, None, None).unwrap();
    assert!(low.len() < high.len());
}

#[test]
fn avif_input_is_unsupported() {
    let input = fixture("input.avif");
    let err = convert(&input, ImageFormat::Png, 80, None, None).unwrap_err();
    assert_eq!(err, ppff_image_conversion::Error::UnsupportedFormat);
}

#[test]
fn garbage_input_is_invalid() {
    let err = convert(b"not an image", ImageFormat::Jpeg, 80, None, None).unwrap_err();
    assert_eq!(err, ppff_image_conversion::Error::InvalidInput);
}

#[test]
fn empty_input_is_invalid() {
    let err = convert(b"", ImageFormat::Jpeg, 80, None, None).unwrap_err();
    assert_eq!(err, ppff_image_conversion::Error::InvalidInput);
}

#[test]
fn zero_dimensions_are_invalid() {
    let input = fixture("input.png");
    let err = convert(&input, ImageFormat::Jpeg, 80, Some(0), None).unwrap_err();
    assert_eq!(err, ppff_image_conversion::Error::InvalidInput);
}

#[test]
fn out_of_range_quality_is_clamped() {
    let input = fixture("input.png");
    let output = convert(&input, ImageFormat::WebP, 255, None, None).unwrap();
    assert!(is_webp(&output));
}

#[test]
fn jpeg_output_for_odd_dimensions() {
    let input = fixture("input.png");
    let output = convert(&input, ImageFormat::Jpeg, 80, Some(63), Some(47)).unwrap();
    assert!(is_jpeg(&output));
}

#[test]
fn scale_resizes_output() {
    let input = fixture("input.png");
    let converter = ImageConverter::new().unwrap();
    let output = converter
        .convert(
            &input,
            ConversionOptions {
                format: ImageFormat::Png,
                scale: Some(0.25),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(png_dimensions(&output), (40, 30));
}

#[test]
fn explicit_dimensions_win_over_scale() {
    let input = fixture("input.png");
    let converter = ImageConverter::new().unwrap();
    let output = converter
        .convert(
            &input,
            ConversionOptions {
                format: ImageFormat::Png,
                width: Some(64),
                scale: Some(0.25),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(png_dimensions(&output), (64, 48));
}

#[test]
fn avif_effort_endpoints_encode() {
    let input = fixture("input.png");
    let converter = ImageConverter::new().unwrap();
    for effort in [0, 100] {
        let output = converter
            .convert(
                &input,
                ConversionOptions {
                    format: ImageFormat::Avif,
                    quality: 70,
                    effort: Some(effort),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(is_avif(&output), "invalid avif at effort {effort}");
    }
}

#[test]
fn max_bytes_limits_jpeg_size() {
    let input = fixture("input.png");
    let converter = ImageConverter::new().unwrap();
    let full = convert(&input, ImageFormat::Jpeg, 95, None, None).unwrap();
    let budget = full.len() * 2 / 3;
    let output = converter
        .convert(
            &input,
            ConversionOptions {
                format: ImageFormat::Jpeg,
                quality: 95,
                max_bytes: Some(budget as u64),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(output.len() <= budget);
    assert!(is_jpeg(&output));
}
