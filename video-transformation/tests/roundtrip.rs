use ppff_video_conversion::VideoStream;
use video_transformation::{
    Error, TransformOperation, TransformOptions, VideoFormat, VideoTransformer,
};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .expect("fixture missing")
}

fn transform(
    input: &[u8],
    operations: Vec<TransformOperation>,
    format: Option<VideoFormat>,
) -> Result<Vec<u8>, Error> {
    VideoTransformer::new()?.transform(
        input,
        TransformOptions {
            operations,
            custom_filters: None,
            format,
            quality: None,
        },
    )
}

fn probe(bytes: &[u8]) -> (u32, u32, Option<f64>, bool) {
    let stream = VideoStream::open(bytes).unwrap();
    let info = stream.info().clone();
    (info.width, info.height, info.duration, info.has_audio)
}

fn is_mp4(data: &[u8]) -> bool {
    data.len() > 12 && &data[4..8] == b"ftyp"
}

fn is_webm(data: &[u8]) -> bool {
    data.len() > 4 && data[..4] == [0x1A, 0x45, 0xDF, 0xA3]
}

fn duration_of(bytes: &[u8]) -> f64 {
    probe(bytes).2.expect("duration missing")
}

#[test]
fn grayscale_keeps_dimensions_and_default_container() {
    let input = fixture("input.mp4");
    let output = transform(&input, vec![TransformOperation::Grayscale], None).unwrap();
    assert!(is_mp4(&output));
    let (width, height, duration, _) = probe(&output);
    assert_eq!((width, height), (320, 180));
    assert!((duration.unwrap() - 2.0).abs() < 0.15);
}

#[test]
fn crop_and_scale_change_dimensions() {
    let input = fixture("input.mp4");
    let output = transform(
        &input,
        vec![
            TransformOperation::Crop {
                x: 0,
                y: 0,
                width: 160,
                height: 90,
            },
            TransformOperation::Scale {
                width: 64,
                height: 64,
            },
        ],
        Some(VideoFormat::WebM),
    )
    .unwrap();
    assert!(is_webm(&output));
    let (width, height, _, _) = probe(&output);
    assert_eq!((width, height), (64, 64));
}

#[test]
fn trim_shortens_the_duration_and_keeps_audio() {
    let input = fixture("input_audio.mp4");
    let output = transform(
        &input,
        vec![TransformOperation::Trim {
            start: 0.5,
            duration: Some(1.0),
        }],
        None,
    )
    .unwrap();
    let duration = duration_of(&output);
    assert!((duration - 1.0).abs() < 0.3, "duration {duration}");
    assert!(probe(&output).3, "audio should survive a trim");
}

#[test]
fn trim_rebases_the_webm_timeline_to_zero() {
    let input = fixture("input_audio.mp4");
    let output = transform(
        &input,
        vec![TransformOperation::Trim {
            start: 0.5,
            duration: Some(1.0),
        }],
        Some(VideoFormat::WebM),
    )
    .unwrap();
    let duration = duration_of(&output);
    assert!((duration - 1.0).abs() < 0.3, "duration {duration}");
}

#[test]
fn speed_changes_duration_and_drops_audio() {
    let input = fixture("input_audio.mp4");

    let faster = transform(
        &input,
        vec![TransformOperation::Speed { factor: 2.0 }],
        None,
    )
    .unwrap();
    let duration = duration_of(&faster);
    assert!((duration - 1.0).abs() < 0.3, "duration {duration}");
    assert!(!probe(&faster).3, "audio must be dropped for speed");

    let slower = transform(
        &input,
        vec![TransformOperation::Speed { factor: 0.5 }],
        None,
    )
    .unwrap();
    let duration = duration_of(&slower);
    assert!((duration - 4.0).abs() < 0.5, "duration {duration}");
}

#[test]
fn reverse_keeps_duration_drops_audio_and_stays_decodable() {
    let input = fixture("input_audio.mp4");
    let output = transform(&input, vec![TransformOperation::Reverse], None).unwrap();
    assert!(is_mp4(&output));
    let (width, height, duration, has_audio) = probe(&output);
    assert_eq!((width, height), (320, 180));
    assert!(!has_audio, "audio must be dropped for reverse");
    assert!((duration.unwrap() - 2.0).abs() < 0.3);

    let mut stream = VideoStream::open(&output).unwrap();
    let mut frames = 0usize;
    while let Some(event) = stream.next_event().unwrap() {
        if matches!(event, ppff_video_conversion::StreamEvent::Video(_)) {
            frames += 1;
        }
    }
    assert!(frames >= 45, "only {frames} frames decoded");
}

#[test]
fn custom_filters_apply_last() {
    let input = fixture("input.mp4");
    let output = VideoTransformer::new()
        .unwrap()
        .transform(
            &input,
            TransformOptions {
                operations: vec![TransformOperation::Grayscale],
                custom_filters: Some("hflip".to_string()),
                format: None,
                quality: None,
            },
        )
        .unwrap();
    assert!((duration_of(&output) - 2.0).abs() < 0.15);
}

#[test]
fn explicit_webm_output_from_mp4_input() {
    let input = fixture("input.mp4");
    let output = transform(&input, vec![], Some(VideoFormat::WebM)).unwrap();
    assert!(is_webm(&output));
    assert!((duration_of(&output) - 2.0).abs() < 0.3);
}

#[test]
fn reverse_on_large_clip_is_rejected() {
    let input = fixture("input_big.mp4");
    let err = transform(&input, vec![TransformOperation::Reverse], None).unwrap_err();
    match err {
        Error::LimitExceeded(msg) => {
            assert!(msg.contains("MiB"), "{msg}");
            assert!(msg.contains("Reverse"), "{msg}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn out_of_bounds_crop_is_invalid() {
    let input = fixture("input.mp4");
    let err = transform(
        &input,
        vec![TransformOperation::Crop {
            x: 300,
            y: 0,
            width: 100,
            height: 100,
        }],
        None,
    )
    .unwrap_err();
    assert_eq!(err, Error::InvalidInput);
}

#[test]
fn empty_input_is_invalid() {
    let err = transform(&[], vec![], None).unwrap_err();
    assert_eq!(err, Error::InvalidInput);
}

#[test]
fn garbage_input_is_invalid() {
    let err = transform(b"not a video", vec![], None).unwrap_err();
    assert_eq!(err, Error::InvalidInput);
}
