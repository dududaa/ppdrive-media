use ppff_video_conversion::{
    ConversionOptions, StreamEvent, VideoConverter, VideoFormat, VideoStream,
};

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
    format: VideoFormat,
    quality: u8,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<Vec<u8>, ppff_video_conversion::Error> {
    let converter = VideoConverter::new()?;
    converter.convert(
        input,
        ConversionOptions {
            format,
            quality,
            width,
            height,
            ..ConversionOptions::default()
        },
    )
}

fn is_mp4(data: &[u8]) -> bool {
    data.len() > 12 && &data[4..8] == b"ftyp"
}

fn has_doctype(data: &[u8], doctype: &[u8]) -> bool {
    data.len() > 4
        && data[..4] == [0x1A, 0x45, 0xDF, 0xA3]
        && data
            .windows(doctype.len())
            .take(64)
            .any(|window| window == doctype)
}

fn is_webm(data: &[u8]) -> bool {
    has_doctype(data, b"webm")
}

fn is_mkv(data: &[u8]) -> bool {
    has_doctype(data, b"matroska")
}

fn is_avi(data: &[u8]) -> bool {
    data.len() > 12 && &data[..4] == b"RIFF" && &data[8..12] == b"AVI "
}

type FormatCheck = (VideoFormat, fn(&[u8]) -> bool);

const ALL_FORMATS: [FormatCheck; 10] = [
    (VideoFormat::Mp4, is_mp4),
    (VideoFormat::WebM, is_webm),
    (VideoFormat::Mov, is_mp4),
    (VideoFormat::Mkv, is_mkv),
    (VideoFormat::Avi, is_avi),
    (VideoFormat::Mp4Av1, is_mp4),
    (VideoFormat::WebMAv1, is_webm),
    (VideoFormat::MkvAv1, is_mkv),
    (VideoFormat::Mp4Hevc, is_mp4),
    (VideoFormat::MovHevc, is_mp4),
];

#[test]
fn mp4_input_roundtrips_to_all_formats() {
    let input = fixture("input.mp4");
    for (format, check) in ALL_FORMATS {
        let output = convert(&input, format, 80, None, None).unwrap();
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
    for (format, _) in ALL_FORMATS {
        let output = convert(&input, format, 80, None, None).unwrap();
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

    let output = convert(&input, VideoFormat::Mp4, 80, Some(160), None).unwrap();
    let stream = VideoStream::open(&output).unwrap();
    assert_eq!((stream.info().width, stream.info().height), (160, 90));

    let output = convert(&input, VideoFormat::Mp4, 80, None, Some(90)).unwrap();
    let stream = VideoStream::open(&output).unwrap();
    assert_eq!((stream.info().width, stream.info().height), (160, 90));

    let output = convert(&input, VideoFormat::Mp4, 80, Some(101), Some(57)).unwrap();
    let stream = VideoStream::open(&output).unwrap();
    assert_eq!((stream.info().width, stream.info().height), (102, 58));
}

#[test]
fn quality_controls_output_size() {
    let input = fixture("input.mp4");
    let low = convert(&input, VideoFormat::Mp4, 10, None, None).unwrap();
    let high = convert(&input, VideoFormat::Mp4, 95, None, None).unwrap();
    assert!(low.len() < high.len());
}

#[test]
fn out_of_range_quality_is_clamped() {
    let input = fixture("input.mp4");
    let output = convert(&input, VideoFormat::Mp4, 255, None, None).unwrap();
    assert!(is_mp4(&output));
}

#[test]
fn extension_mapping_covers_plain_containers() {
    assert_eq!(VideoFormat::from_extension("m4v"), Some(VideoFormat::Mp4));
    assert_eq!(VideoFormat::from_extension("webm"), Some(VideoFormat::WebM));
    assert_eq!(VideoFormat::from_extension("MKV"), Some(VideoFormat::Mkv));
    assert_eq!(VideoFormat::from_extension("mov"), Some(VideoFormat::Mov));
    assert_eq!(VideoFormat::from_extension("avi"), Some(VideoFormat::Avi));
    assert_eq!(VideoFormat::from_extension("gif"), None);
}

#[test]
fn garbage_input_is_invalid() {
    let err = convert(b"not a video", VideoFormat::Mp4, 80, None, None).unwrap_err();
    assert_eq!(err, ppff_video_conversion::Error::InvalidInput);
}

#[test]
fn empty_input_is_invalid() {
    let err = convert(b"", VideoFormat::Mp4, 80, None, None).unwrap_err();
    assert_eq!(err, ppff_video_conversion::Error::InvalidInput);
}
