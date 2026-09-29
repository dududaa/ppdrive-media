use video_compression::{
    CompressionOptions, StreamEvent, VideoCompressor, VideoFormat, VideoStream,
};

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
    format: VideoFormat,
    quality: u8,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<Vec<u8>, video_compression::Error> {
    let compressor = VideoCompressor::new()?;
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

fn is_mp4(data: &[u8]) -> bool {
    data.len() > 12 && &data[4..8] == b"ftyp"
}

fn is_webm(data: &[u8]) -> bool {
    data.len() > 4 && data[..4] == [0x1A, 0x45, 0xDF, 0xA3]
}

#[test]
fn mp4_input_roundtrips_to_both_formats() {
    let input = fixture("input.mp4");
    for (format, check) in [
        (VideoFormat::Mp4, is_mp4 as fn(&[u8]) -> bool),
        (VideoFormat::WebM, is_webm),
    ] {
        let output = compress(&input, format, 80, None, None).unwrap();
        assert!(check(&output), "invalid output for {format:?}");
        let stream = VideoStream::open(&output).unwrap();
        assert_eq!(stream.info().width, 320);
        assert_eq!(stream.info().height, 180);
        let duration = stream.info().duration.expect("duration");
        assert!((duration - 2.0).abs() < 0.15, "bad duration {duration}");
    }
}

#[test]
fn outputs_are_fully_decodable() {
    let input = fixture("input.mp4");
    for format in [VideoFormat::Mp4, VideoFormat::WebM] {
        let output = compress(&input, format, 80, None, None).unwrap();
        let mut stream = VideoStream::open(&output).unwrap();
        let mut frames = 0usize;
        while let Some(event) = stream.next_event().unwrap() {
            if let StreamEvent::Video(_) = event {
                frames += 1;
            }
        }
        assert!(frames >= 45, "{format:?}: only {frames} frames decoded");
    }
}

#[test]
fn resize_preserves_dimensions_and_aspect() {
    let input = fixture("input.mp4");

    let output = compress(&input, VideoFormat::Mp4, 80, Some(160), None).unwrap();
    let stream = VideoStream::open(&output).unwrap();
    assert_eq!((stream.info().width, stream.info().height), (160, 90));

    let output = compress(&input, VideoFormat::Mp4, 80, None, Some(90)).unwrap();
    let stream = VideoStream::open(&output).unwrap();
    assert_eq!((stream.info().width, stream.info().height), (160, 90));

    let output = compress(&input, VideoFormat::Mp4, 80, Some(101), Some(57)).unwrap();
    let stream = VideoStream::open(&output).unwrap();
    assert_eq!((stream.info().width, stream.info().height), (102, 58));
}

#[test]
fn quality_controls_output_size() {
    let input = fixture("input.mp4");
    let low = compress(&input, VideoFormat::Mp4, 10, None, None).unwrap();
    let high = compress(&input, VideoFormat::Mp4, 95, None, None).unwrap();
    assert!(low.len() < high.len());
}

#[test]
fn out_of_range_quality_is_clamped() {
    let input = fixture("input.mp4");
    let output = compress(&input, VideoFormat::Mp4, 255, None, None).unwrap();
    assert!(is_mp4(&output));
}

#[test]
fn m4v_extension_maps_to_mp4() {
    assert_eq!(VideoFormat::from_extension("m4v"), Some(VideoFormat::Mp4));
    assert_eq!(VideoFormat::from_extension("MKV"), Some(VideoFormat::WebM));
    assert_eq!(VideoFormat::from_extension("avi"), None);
}

#[test]
fn garbage_input_is_invalid() {
    let err = compress(b"not a video", VideoFormat::Mp4, 80, None, None).unwrap_err();
    assert_eq!(err, video_compression::Error::InvalidInput);
}

#[test]
fn empty_input_is_invalid() {
    let err = compress(b"", VideoFormat::Mp4, 80, None, None).unwrap_err();
    assert_eq!(err, video_compression::Error::InvalidInput);
}
