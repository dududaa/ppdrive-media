pub mod ladder;
pub mod options;
pub mod session;

mod error;
mod ffi;

pub use error::Error;
pub use options::{RenditionSpec, StreamingOptions, StreamingProtocol};

use std::path::{Path, PathBuf};

use audio_conversion::{DecodedAudio, EncodedPacket, EncoderParams};
use video_conversion::{StreamEvent, VideoEncoder, VideoFormat, VideoStream, VideoStreamInfo};

use crate::error::Error as StreamError;

/// A finished HLS/DASH package on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamingOutput {
    /// The master playlist (`master.m3u8` for HLS, `manifest.mpd`
    /// for DASH) inside the output directory.
    pub playlist: PathBuf,
    /// Every file the muxer wrote in the output directory (playlists,
    /// init segments and media segments), sorted by name.
    pub files: Vec<PathBuf>,
}

/// Packages media into HLS or DASH renditions.
///
/// ```no_run
/// use media_streaming::{MediaStreamer, StreamingOptions, StreamingProtocol};
///
/// # let input: Vec<u8> = Vec::new();
/// let streamer = MediaStreamer::new()?;
/// let output = streamer.stream(
///     &input,
///     std::path::Path::new("./out"),
///     &StreamingOptions::default(),
/// )?;
/// assert!(output.playlist.ends_with("master.m3u8"));
/// # Ok::<(), media_streaming::Error>(())
/// ```
pub struct MediaStreamer;

impl MediaStreamer {
    /// Checks that the linked FFmpeg build provides the `hls` and
    /// `dash` muxers.
    ///
    /// FFmpeg's global log level is set to `AV_LOG_ERROR` on first
    /// call and never raised above it.
    pub fn new() -> Result<MediaStreamer, StreamError> {
        unsafe {
            if ffi::av_log_get_level() > ffi::AV_LOG_ERROR as i32 {
                ffi::av_log_set_level(ffi::AV_LOG_ERROR as i32);
            }
        }
        for name in ["hls", "dash"] {
            let c = ffi::to_cstr(name)?;
            let found =
                unsafe { ffi::av_guess_format(c.as_ptr(), std::ptr::null(), std::ptr::null()) };
            if found.is_null() {
                return Err(StreamError::UnsupportedFormat);
            }
        }
        Ok(MediaStreamer)
    }

    /// Packages `input` (any container/codec FFmpeg can decode, probed
    /// from content) into `output_dir`, which is created when missing.
    ///
    /// Video inputs produce one rendition per
    /// [`StreamingOptions::renditions`] entry (or an auto ladder from
    /// the source resolution) plus one shared AAC audio rendition when
    /// the input carries audio. Audio-only inputs produce a single
    /// audio rendition. Returns the master playlist and the list of
    /// files written.
    pub fn stream(
        &self,
        input: &[u8],
        output_dir: &Path,
        options: &StreamingOptions,
    ) -> Result<StreamingOutput, StreamError> {
        options.validate()?;
        if input.is_empty() {
            return Err(StreamError::InvalidInput);
        }
        std::fs::create_dir_all(output_dir).map_err(StreamError::io)?;

        let playlist = match options.protocol {
            options::StreamingProtocol::Hls => output_dir.join("master.m3u8"),
            options::StreamingProtocol::Dash => output_dir.join("manifest.mpd"),
        };

        match VideoStream::open(input) {
            Ok(stream) => package_video(input, stream, &playlist, output_dir, options),
            Err(video_conversion::Error::UnsupportedFormat) => {
                package_audio(input, &playlist, output_dir, options)
            }
            Err(err) => Err(crate::ffi::from_video(err)),
        }
    }

    /// Reads the file at `input_path` and packages it like
    /// [`MediaStreamer::stream`].
    pub fn stream_file(
        &self,
        input_path: &Path,
        output_dir: &Path,
        options: &StreamingOptions,
    ) -> Result<StreamingOutput, StreamError> {
        let input = std::fs::read(input_path).map_err(StreamError::io)?;
        self.stream(&input, output_dir, options)
    }
}

impl Default for MediaStreamer {
    fn default() -> Self {
        MediaStreamer
    }
}

/// Buffered audio for one shared rendition: open-encoder params plus
/// every encoded packet, in pts order.
struct BufferedAudio {
    params: EncoderParams,
    packets: Vec<EncodedPacket>,
    bitrate: u64,
}

fn package_video(
    input: &[u8],
    mut stream: VideoStream<'_>,
    playlist: &Path,
    output_dir: &Path,
    options: &options::StreamingOptions,
) -> Result<StreamingOutput, StreamError> {
    let info: VideoStreamInfo = stream.info().clone();
    let fps = if info.frame_rate_num > 0 && info.frame_rate_den > 0 {
        f64::from(info.frame_rate_num) / f64::from(info.frame_rate_den)
    } else {
        0.0
    };
    let rungs = ladder::resolve(options, info.width, info.height, fps);
    let keyframe_interval = if fps > 0.0 {
        ((fps * f64::from(options.segment_duration)).round() as u32).max(1)
    } else {
        0
    };
    let keyframe_interval = (keyframe_interval > 0).then_some(keyframe_interval);

    // The HLS muxer sets `AVFMT_GLOBALHEADER`, so both protocols need
    // global headers: avcC extradata feeds the master playlist's
    // `CODECS` attribute (hlsenc drops the whole attribute when the
    // video extradata is missing).
    let global_headers = true;
    let encoders: Vec<VideoEncoder> = rungs
        .iter()
        .map(|rung| {
            VideoEncoder::packetized(
                VideoFormat::Mp4,
                rung.quality,
                None,
                None,
                keyframe_interval,
                rung.width,
                rung.height,
                &info,
                global_headers,
            )
        })
        .collect::<Result<_, video_conversion::Error>>()
        .map_err(crate::ffi::from_video)?;

    let audio = if info.has_audio {
        Some(buffer_audio(input, options, global_headers)?)
    } else {
        None
    };

    let mut session = session::SegmentSession::new(options.protocol, playlist)?;
    let mut video_ids = Vec::with_capacity(encoders.len());
    for (encoder, rung) in encoders.iter().zip(&rungs) {
        video_ids.push(session.add_video_stream(encoder.params(), rung.bitrate)?);
    }
    let hls = options.protocol == options::StreamingProtocol::Hls;
    let mut audio_ids = Vec::new();
    if let Some(audio) = &audio {
        // The HLS muxer rejects one elementary stream in two variants,
        // so each variant carries its own copy of the audio; DASH keeps
        // a single stream shared via `adaptation_sets`.
        let copies = if hls { video_ids.len().max(1) } else { 1 };
        for _ in 0..copies {
            audio_ids.push(session.add_audio_stream(&audio.params, audio.bitrate)?);
        }
    }
    session.write_header(&header_options(
        options,
        playlist,
        video_ids.len(),
        !audio_ids.is_empty(),
    ))?;

    let mut audio_queue = audio
        .map(|audio| {
            audio
                .packets
                .into_iter()
                .map(|packet| (packet_seconds(&packet), packet))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut encoders = encoders;
    while let Some(event) = stream.next_event().map_err(crate::ffi::from_video)? {
        if let StreamEvent::Video(mut frame) = event {
            for (index, encoder) in encoders.iter_mut().enumerate() {
                encoder
                    .encode_frame(&mut frame)
                    .map_err(crate::ffi::from_video)?;
                for packet in encoder.take_pending() {
                    flush_audio_upto(
                        &mut audio_queue,
                        packet_seconds_video(&packet),
                        &mut session,
                        &audio_ids,
                    );
                    session.write_video(video_ids[index], &packet)?;
                }
            }
        }
    }
    for (index, encoder) in encoders.iter_mut().enumerate() {
        encoder.finish_drain().map_err(crate::ffi::from_video)?;
        for packet in encoder.take_pending() {
            flush_audio_upto(&mut audio_queue, f64::MAX, &mut session, &audio_ids);
            session.write_video(video_ids[index], &packet)?;
        }
    }
    flush_audio_upto(&mut audio_queue, f64::MAX, &mut session, &audio_ids);
    session.finish()?;
    collect_output(playlist, output_dir)
}

fn package_audio(
    input: &[u8],
    playlist: &Path,
    output_dir: &Path,
    options: &options::StreamingOptions,
) -> Result<StreamingOutput, StreamError> {
    let audio = buffer_audio(input, options, true)?;

    let mut session = session::SegmentSession::new(options.protocol, playlist)?;
    let audio_id = session.add_audio_stream(&audio.params, audio.bitrate)?;
    session.write_header(&header_options(options, playlist, 0, true))?;
    for packet in &audio.packets {
        session.write_audio(audio_id, packet)?;
    }
    session.finish()?;
    collect_output(playlist, output_dir)
}

fn buffer_audio(
    input: &[u8],
    options: &options::StreamingOptions,
    global_headers: bool,
) -> Result<BufferedAudio, StreamError> {
    let decoded = DecodedAudio::decode(input).map_err(crate::ffi::from_audio)?;
    let mut params: Option<EncoderParams> = None;
    let mut packets = Vec::new();
    decoded
        .encode_to(
            audio_conversion::AudioFormat::Aac,
            options.quality,
            global_headers,
            &mut |packet_params, packet| {
                if params.is_none() {
                    params = Some(packet_params.clone());
                }
                packets.push(packet.clone());
            },
        )
        .map_err(crate::ffi::from_audio)?;
    let params = params.ok_or(StreamError::InvalidInput)?;
    let bitrate =
        audio_conversion::bitrate_for_quality(audio_conversion::AudioFormat::Aac, options.quality)
            .unwrap_or(128_000);
    Ok(BufferedAudio {
        params,
        packets,
        bitrate: u64::from(bitrate),
    })
}

fn packet_seconds(packet: &EncodedPacket) -> f64 {
    packet.pts as f64 * f64::from(packet.tb_num) / f64::from(packet.tb_den)
}

fn packet_seconds_video(packet: &video_conversion::VideoPacket) -> f64 {
    packet.pts as f64 * f64::from(packet.tb_num) / f64::from(packet.tb_den)
}

fn flush_audio_upto(
    queue: &mut Vec<(f64, EncodedPacket)>,
    seconds: f64,
    session: &mut session::SegmentSession,
    audio_ids: &[usize],
) {
    while !queue.is_empty() && queue[0].0 <= seconds {
        let (_, packet) = queue.remove(0);
        for &audio_id in audio_ids {
            let _ = session.write_audio(audio_id, &packet);
        }
    }
}

fn header_options(
    options: &options::StreamingOptions,
    playlist: &Path,
    video_renditions: usize,
    has_audio: bool,
) -> Vec<(String, String)> {
    match options.protocol {
        options::StreamingProtocol::Hls => {
            let map = match (video_renditions, has_audio) {
                (0, true) => "a:0".to_string(),
                (0, false) => String::new(),
                (n, false) => (0..n)
                    .map(|i| format!("v:{i}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                (n, true) => (0..n)
                    .map(|i| format!("v:{i},a:{i}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            };
            session::hls_header_options(options.segment_duration, &map, playlist)
        }
        options::StreamingProtocol::Dash => {
            session::dash_header_options(options.segment_duration, video_renditions > 0, has_audio)
        }
    }
}

fn collect_output(playlist: &Path, output_dir: &Path) -> Result<StreamingOutput, StreamError> {
    const EXTS: [&str; 6] = ["m3u8", "mpd", "ts", "m4s", "mp4", "aac"];
    let mut files = Vec::new();
    let entries = std::fs::read_dir(output_dir).map_err(StreamError::io)?;
    for entry in entries {
        let entry = entry.map_err(StreamError::io)?;
        let path = entry.path();
        let keep = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| EXTS.contains(&ext));
        if keep {
            files.push(path);
        }
    }
    files.sort();
    Ok(StreamingOutput {
        playlist: playlist.to_path_buf(),
        files,
    })
}
