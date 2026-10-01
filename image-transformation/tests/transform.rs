use ppff_image_conversion::{Frame, ImageFormat};
use image_transformation::{Error, ImageTransformer, TransformOperation, TransformOptions};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .expect("fixture missing")
}

fn transform_with(input: &[u8], options: TransformOptions) -> Result<Vec<u8>, Error> {
    ImageTransformer::new()?.transform(input, options)
}

fn transform(input: &[u8], operations: Vec<TransformOperation>) -> Result<Vec<u8>, Error> {
    transform_with(
        input,
        TransformOptions {
            operations,
            custom_filters: None,
            format: None,
            quality: None,
        },
    )
}

fn crop(width: u32, height: u32, x: u32, y: u32) -> TransformOperation {
    TransformOperation::Crop {
        x,
        y,
        width,
        height,
    }
}

fn dims(data: &[u8]) -> (u32, u32) {
    let frame = Frame::decode(data).expect("output does not decode");
    (frame.width(), frame.height())
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

#[test]
fn identity_transform_keeps_input() {
    let input = fixture("input.png");
    let output = transform(&input, vec![]).unwrap();
    assert!(is_png(&output));
    assert_eq!(dims(&output), (160, 120));
}

#[test]
fn crop_reduces_dimensions() {
    let input = fixture("input.png");
    let output = transform(&input, vec![crop(80, 60, 40, 30)]).unwrap();
    assert!(is_png(&output));
    assert_eq!(dims(&output), (80, 60));
}

#[test]
fn crop_at_origin_keeps_dimensions() {
    let input = fixture("input.png");
    let output = transform(&input, vec![crop(160, 120, 0, 0)]).unwrap();
    assert_eq!(dims(&output), (160, 120));
}

#[test]
fn rotate_90_swaps_dimensions() {
    let input = fixture("input.png");
    let output = transform(&input, vec![TransformOperation::Rotate { degrees: 90 }]).unwrap();
    assert_eq!(dims(&output), (120, 160));
}

#[test]
fn rotate_180_keeps_dimensions() {
    let input = fixture("input.png");
    let output = transform(&input, vec![TransformOperation::Rotate { degrees: 180 }]).unwrap();
    assert_eq!(dims(&output), (160, 120));
}

#[test]
fn rotate_270_swaps_dimensions() {
    let input = fixture("input.png");
    let output = transform(&input, vec![TransformOperation::Rotate { degrees: 270 }]).unwrap();
    assert_eq!(dims(&output), (120, 160));
}

#[test]
fn flip_keeps_dimensions() {
    let input = fixture("input.png");
    let output = transform(
        &input,
        vec![TransformOperation::Flip {
            horizontal: true,
            vertical: true,
        }],
    )
    .unwrap();
    assert_eq!(dims(&output), (160, 120));
}

#[test]
fn pad_adds_dimensions() {
    let input = fixture("input.png");
    let output = transform(
        &input,
        vec![TransformOperation::Pad {
            left: 5,
            top: 6,
            right: 7,
            bottom: 8,
            color: "#102030".to_string(),
        }],
    )
    .unwrap();
    assert_eq!(dims(&output), (160 + 5 + 7, 120 + 6 + 8));
}

#[test]
fn grayscale_keeps_dimensions() {
    let input = fixture("input.png");
    let output = transform(&input, vec![TransformOperation::Grayscale]).unwrap();
    assert_eq!(dims(&output), (160, 120));
}

#[test]
fn adjust_encodes() {
    let input = fixture("input.png");
    let output = transform(
        &input,
        vec![TransformOperation::Adjust {
            brightness: 0.1,
            contrast: 1.3,
            saturation: 0.4,
        }],
    )
    .unwrap();
    assert!(is_png(&output));
    assert_eq!(dims(&output), (160, 120));
}

#[test]
fn adjust_converts_yuv_input() {
    // JPEG decodes to YUV420; the adjust stage forces RGB via
    // `format=` before lutrgb and must still encode.
    let input = fixture("input.jpg");
    let output = transform(
        &input,
        vec![TransformOperation::Adjust {
            brightness: 0.05,
            contrast: 1.2,
            saturation: 1.1,
        }],
    )
    .unwrap();
    assert!(is_jpeg(&output));
    assert_eq!(dims(&output), (160, 120));
}

#[test]
fn adjust_preserves_alpha() {
    let input = fixture("input_alpha.png");
    let output = transform(
        &input,
        vec![TransformOperation::Adjust {
            brightness: 0.0,
            contrast: 1.1,
            saturation: 1.0,
        }],
    )
    .unwrap();
    let frame = Frame::decode(&output).unwrap();
    assert!(frame.has_alpha(), "alpha lost through adjust");
}

#[test]
fn blur_keeps_dimensions() {
    let input = fixture("input.png");
    let output = transform(&input, vec![TransformOperation::Blur { sigma: 1.5 }]).unwrap();
    assert_eq!(dims(&output), (160, 120));
}

#[test]
fn sharpen_keeps_dimensions() {
    let input = fixture("input.png");
    let output = transform(&input, vec![TransformOperation::Sharpen { amount: 0.8 }]).unwrap();
    assert_eq!(dims(&output), (160, 120));
}

#[test]
fn scale_exact_dimensions() {
    let input = fixture("input.png");
    let output = transform(
        &input,
        vec![TransformOperation::Scale {
            width: 64,
            height: 48,
        }],
    )
    .unwrap();
    assert_eq!(dims(&output), (64, 48));
}

#[test]
fn combined_operations_apply_in_order() {
    let input = fixture("input.png");
    let output = transform(
        &input,
        vec![
            crop(80, 60, 40, 30),
            TransformOperation::Rotate { degrees: 90 },
            TransformOperation::Grayscale,
        ],
    )
    .unwrap();
    // crop 80x60, then rotate 90 → 60x80
    assert_eq!(dims(&output), (60, 80));
}

#[test]
fn custom_filter_string_applied() {
    let input = fixture("input.png");
    let output = transform_with(
        &input,
        TransformOptions {
            operations: vec![TransformOperation::Grayscale],
            custom_filters: Some("vignette=PI/5".to_string()),
            format: None,
            quality: None,
        },
    )
    .unwrap();
    assert_eq!(dims(&output), (160, 120));
}

#[test]
fn invalid_custom_filter_is_an_error() {
    let input = fixture("input.png");
    let err = transform_with(
        &input,
        TransformOptions {
            operations: vec![],
            custom_filters: Some("doesnotexist=1".to_string()),
            format: None,
            quality: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, Error::FfmpegError(_) | Error::InvalidInput));
}

#[test]
fn invalid_operations_are_rejected_before_ffmpeg() {
    let input = fixture("input.png");
    let cases = vec![
        TransformOperation::Rotate { degrees: 45 },
        crop(100, 100, 100, 100),
        TransformOperation::Scale {
            width: 0,
            height: 10,
        },
        TransformOperation::Blur { sigma: 0.0 },
        TransformOperation::Adjust {
            brightness: f32::NAN,
            contrast: 1.0,
            saturation: 1.0,
        },
        TransformOperation::Flip {
            horizontal: false,
            vertical: false,
        },
        TransformOperation::Pad {
            left: 1,
            top: 0,
            right: 0,
            bottom: 0,
            color: String::new(),
        },
    ];
    for op in cases {
        let err = transform(&input, vec![op.clone()]).unwrap_err();
        assert_eq!(err, Error::InvalidInput, "expected InvalidInput for {op:?}");
    }
}

#[test]
fn format_is_kept_per_input() {
    for (name, check) in [
        ("input.png", is_png as fn(&[u8]) -> bool),
        ("input.jpg", is_jpeg),
        ("input.webp", is_webp),
    ] {
        let input = fixture(name);
        let output = transform(&input, vec![crop(80, 60, 40, 30)]).unwrap();
        assert!(check(&output), "input format not kept for {name}");
    }
}

#[test]
fn format_override_png_to_jpeg() {
    let input = fixture("input.png");
    let output = transform_with(
        &input,
        TransformOptions {
            operations: vec![TransformOperation::Grayscale],
            custom_filters: None,
            format: Some(ImageFormat::Jpeg),
            quality: None,
        },
    )
    .unwrap();
    assert!(is_jpeg(&output));
}

#[test]
fn odd_crop_to_jpeg_is_even_fixed() {
    let input = fixture("input.png");
    let output = transform_with(
        &input,
        TransformOptions {
            operations: vec![crop(63, 47, 0, 0)],
            custom_filters: None,
            format: Some(ImageFormat::Jpeg),
            quality: None,
        },
    )
    .unwrap();
    assert!(is_jpeg(&output));
    assert_eq!(dims(&output), (64, 48));
}

#[test]
fn quality_is_clamped_to_100() {
    let input = fixture("input.png");
    let output = transform_with(
        &input,
        TransformOptions {
            operations: vec![],
            custom_filters: None,
            format: Some(ImageFormat::WebP),
            quality: Some(255),
        },
    )
    .unwrap();
    assert!(is_webp(&output));
}

#[test]
fn jpeg_quality_controls_output_size() {
    let input = fixture("input.png");
    let low = transform_with(
        &input,
        TransformOptions {
            operations: vec![crop(80, 60, 40, 30)],
            custom_filters: None,
            format: Some(ImageFormat::Jpeg),
            quality: Some(10),
        },
    )
    .unwrap();
    let high = transform_with(
        &input,
        TransformOptions {
            operations: vec![crop(80, 60, 40, 30)],
            custom_filters: None,
            format: Some(ImageFormat::Jpeg),
            quality: Some(95),
        },
    )
    .unwrap();
    assert!(low.len() < high.len());
}

#[test]
fn alpha_png_survives_transformation() {
    let input = fixture("input_alpha.png");
    let output = transform(&input, vec![crop(10, 10, 1, 1)]).unwrap();
    let frame = Frame::decode(&output).unwrap();
    assert!(frame.has_alpha(), "alpha channel lost");
    assert_eq!(frame.source_format(), Some(ImageFormat::Png));
}

#[test]
fn jpeg_input_transforms() {
    let input = fixture("input.jpg");
    let output = transform(&input, vec![TransformOperation::Rotate { degrees: 90 }]).unwrap();
    assert!(is_jpeg(&output), "jpeg format not kept");
    let (w, h) = dims(&output);
    assert_eq!((w, h), (120, 160));
}

#[test]
fn webp_input_transforms() {
    let input = fixture("input.webp");
    let output = transform(&input, vec![TransformOperation::Grayscale]).unwrap();
    assert!(is_webp(&output), "webp format not kept");
    assert_eq!(dims(&output), (160, 120));
}

#[test]
fn empty_input_is_invalid() {
    let err = transform(b"", vec![]).unwrap_err();
    assert_eq!(err, Error::InvalidInput);
}

#[test]
fn garbage_input_is_invalid() {
    let err = transform(b"not an image", vec![]).unwrap_err();
    assert_eq!(err, Error::InvalidInput);
}

#[test]
fn avif_input_is_unsupported() {
    let input = fixture("input.avif");
    let err = transform(&input, vec![]).unwrap_err();
    assert_eq!(err, Error::UnsupportedFormat);
}
