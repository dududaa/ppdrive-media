use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use media_streaming::{MediaStreamer, RenditionSpec, StreamingOptions, StreamingProtocol};

/// Unique temp directory that cleans itself up on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "ppdrive-media-streaming-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(path).unwrap()
}

/// 2-second 440 Hz stereo WAV (PCM s16le).
fn synthetic_wav(sample_rate: u32, channels: u16, seconds: f64) -> Vec<u8> {
    let samples = (f64::from(sample_rate) * seconds) as usize;
    let data_len = samples * channels as usize * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for i in 0..samples {
        for ch in 0..channels {
            let t = i as f64 / f64::from(sample_rate);
            let sample =
                0.5 * (std::f64::consts::TAU * 440.0 * t).sin() * if ch == 0 { 1.0 } else { 0.7 };
            out.extend_from_slice(&((sample * 20000.0) as i16).to_le_bytes());
        }
    }
    out
}

fn hls_options() -> StreamingOptions {
    StreamingOptions {
        protocol: StreamingProtocol::Hls,
        segment_duration: 1,
        ..StreamingOptions::default()
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn count_ts(files: &[PathBuf]) -> usize {
    files
        .iter()
        .filter(|f| f.extension().and_then(|e| e.to_str()) == Some("ts"))
        .count()
}

#[test]
fn hls_single_video_rendition_writes_master_variant_and_ts_segments() {
    let dir = TempDir::new("hls-single");
    let out = dir.path().join("out");
    let streamer = MediaStreamer::new().unwrap();
    let output = streamer
        .stream(&fixture("input.mp4"), &out, &hls_options())
        .unwrap();

    assert_eq!(output.playlist, out.join("master.m3u8"));
    assert!(output.playlist.is_file(), "master playlist missing");
    assert!(output.files.contains(&output.playlist));

    let master = read(&output.playlist);
    assert!(master.starts_with("#EXTM3U"), "{master}");
    assert_eq!(master.matches("#EXT-X-STREAM-INF").count(), 1, "{master}");
    assert!(master.contains("stream_0.m3u8"), "{master}");

    let variant = read(&out.join("stream_0.m3u8"));
    assert!(variant.contains("#EXTINF:"), "{variant}");
    assert!(variant.contains("#EXT-X-ENDLIST"), "{variant}");
    assert!(variant.contains("seg_0_"), "{variant}");

    assert!(count_ts(&output.files) >= 2, "{:?}", output.files);
    let segment = output
        .files
        .iter()
        .find(|f| f.extension().and_then(|e| e.to_str()) == Some("ts"))
        .unwrap();
    let head = std::fs::read(segment).unwrap();
    assert_eq!(head.first(), Some(&0x47), "MPEG-TS sync byte missing");
}

#[test]
fn hls_multi_rendition_with_audio_lists_every_variant() {
    let dir = TempDir::new("hls-multi");
    let out = dir.path().join("out");
    let options = StreamingOptions {
        renditions: Some(vec![
            RenditionSpec {
                scale: Some(0.5),
                ..RenditionSpec::default()
            },
            RenditionSpec {
                scale: Some(1.0),
                ..RenditionSpec::default()
            },
            RenditionSpec {
                scale: Some(2.0),
                ..RenditionSpec::default()
            },
        ]),
        ..hls_options()
    };
    let streamer = MediaStreamer::new().unwrap();
    let output = streamer
        .stream(&fixture("input_audio.mp4"), &out, &options)
        .unwrap();

    let master = read(&output.playlist);
    assert_eq!(master.matches("#EXT-X-STREAM-INF").count(), 3, "{master}");
    assert!(master.contains("RESOLUTION=160x90"), "{master}");
    assert!(master.contains("RESOLUTION=320x180"), "{master}");
    assert!(master.contains("RESOLUTION=640x360"), "{master}");
    assert!(master.contains("mp4a.40.2"), "{master}");

    for index in 0..3 {
        let variant = read(&out.join(format!("stream_{index}.m3u8")));
        assert!(variant.contains("#EXTINF:"), "stream_{index}: {variant}");
        assert!(variant.contains("EXT-X-ENDLIST"), "stream_{index}");
    }
    assert!(count_ts(&output.files) >= 3, "{:?}", output.files);
}

#[test]
fn hls_audio_only_packages_master_and_segments() {
    let dir = TempDir::new("hls-audio");
    let out = dir.path().join("out");
    let streamer = MediaStreamer::new().unwrap();
    let output = streamer
        .stream(&synthetic_wav(44_100, 2, 2.0), &out, &hls_options())
        .unwrap();

    let master = read(&output.playlist);
    assert_eq!(master.matches("#EXT-X-STREAM-INF").count(), 1, "{master}");
    assert!(master.contains("mp4a.40.2"), "{master}");
    assert!(!master.contains("RESOLUTION"), "{master}");
    assert!(read(&out.join("stream_0.m3u8")).contains("#EXTINF:"));
    assert!(count_ts(&output.files) >= 1, "{:?}", output.files);
}

#[test]
fn dash_video_and_audio_write_manifest_init_and_media_segments() {
    let dir = TempDir::new("dash-video");
    let out = dir.path().join("out");
    let options = StreamingOptions {
        protocol: StreamingProtocol::Dash,
        segment_duration: 1,
        ..StreamingOptions::default()
    };
    let streamer = MediaStreamer::new().unwrap();
    let output = streamer
        .stream(&fixture("input_audio.mp4"), &out, &options)
        .unwrap();

    assert_eq!(output.playlist, out.join("manifest.mpd"));
    let mpd = read(&output.playlist);
    assert!(mpd.contains("<MPD"), "{mpd}");
    assert!(mpd.matches("<Representation").count() >= 2, "{mpd}");
    assert!(mpd.contains("video/mp4"), "{mpd}");
    assert!(mpd.contains("audio/mp4"), "{mpd}");

    let has_ext = |ext: &str| {
        output
            .files
            .iter()
            .any(|f| f.extension().and_then(|e| e.to_str()) == Some(ext))
    };
    assert!(has_ext("m4s"), "{:?}", output.files);
    let inits = output
        .files
        .iter()
        .filter(|f| {
            f.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("init-stream"))
        })
        .count();
    assert!(inits >= 2, "{:?}", output.files);
    let chunks = output
        .files
        .iter()
        .filter(|f| {
            f.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("chunk-stream"))
        })
        .count();
    assert!(chunks >= 2, "{:?}", output.files);
}

#[test]
fn dash_audio_only_writes_manifest() {
    let dir = TempDir::new("dash-audio");
    let out = dir.path().join("out");
    let options = StreamingOptions {
        protocol: StreamingProtocol::Dash,
        segment_duration: 1,
        ..StreamingOptions::default()
    };
    let streamer = MediaStreamer::new().unwrap();
    let output = streamer
        .stream(&synthetic_wav(44_100, 1, 2.0), &out, &options)
        .unwrap();

    let mpd = read(&output.playlist);
    assert!(mpd.contains("audio/mp4"), "{mpd}");
    assert!(!mpd.contains("video/mp4"), "{mpd}");
    assert!(output.files.iter().any(|f| {
        f.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("chunk-stream"))
    }));
}

#[test]
fn stream_file_creates_missing_output_directory() {
    let dir = TempDir::new("stream-file");
    let out = dir.path().join("nested/deeper/out");
    assert!(!out.exists());
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/input.mp4");
    let streamer = MediaStreamer::new().unwrap();
    let output = streamer
        .stream_file(&fixture_path, &out, &hls_options())
        .unwrap();
    assert!(output.playlist.is_file());
    assert!(out.is_dir());
}
