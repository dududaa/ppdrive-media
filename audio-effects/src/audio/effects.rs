use super::filters;
use super::options::EffectOptions;
use crate::ffi::{FilterGraph, avfilter_get_by_name};
use ppff_audio_conversion::{AudioFormat, DecodedAudio, Error};
use std::ffi::c_void;

/// FFmpeg-backed audio effects runner.
///
/// Stateless: each [`apply`](AudioEffects::apply) call decodes, builds
/// one filter graph, runs every frame through the graph and encodes —
/// no intermediate PCM buffers in Rust beyond the decoded frames
/// themselves.
///
/// # Example
///
/// ```no_run
/// use audio_effects::{AudioEffects, EffectOperation, EffectOptions};
///
/// # let input_bytes: Vec<u8> = Vec::new();
/// let effects = AudioEffects::new()?;
/// let output = effects.apply(
///     &input_bytes,
///     EffectOptions {
///         operations: vec![
///             EffectOperation::Trim { start_secs: 0.5, end_secs: 10.0 },
///             EffectOperation::Volume { gain_db: -3.0 },
///             EffectOperation::Normalize { target_lufs: -16.0 },
///         ],
///         ..Default::default()
///     },
/// )?;
/// # Ok::<(), audio_effects::Error>(())
/// ```
pub struct AudioEffects;

impl AudioEffects {
    /// Lowers FFmpeg's global log level to `AV_LOG_ERROR` (never
    /// raised above it afterwards) and verifies that libavfilter
    /// exposes the `abuffer`/`abuffersink` filters the pipeline
    /// needs.
    pub fn new() -> Result<AudioEffects, Error> {
        unsafe {
            if crate::ffi::av_log_get_level() > crate::ffi::AV_LOG_ERROR as i32 {
                crate::ffi::av_log_set_level(crate::ffi::AV_LOG_ERROR as i32);
            }
            if avfilter_get_by_name(c"abuffer".as_ptr()).is_null()
                || avfilter_get_by_name(c"abuffersink".as_ptr()).is_null()
            {
                return Err(Error::FfmpegError(
                    "libavfilter abuffer/abuffersink filters unavailable".to_string(),
                ));
            }
        }
        Ok(AudioEffects)
    }

    /// Applies effects to one encoded audio stream.
    ///
    /// Pipeline: `DecodedAudio::decode` → `abuffer → chain →
    /// abuffersink` → [`DecodedAudio::encode`] (WAV by default; the
    /// rate/channel rules of [`ppff_audio_conversion::ConversionOptions`]
    /// apply when another format is requested).
    ///
    /// The output format is `options.format`, defaulting to
    /// [`AudioFormat::Wav`]; quality defaults to 80.
    pub fn apply(&self, input: &[u8], options: EffectOptions) -> Result<Vec<u8>, Error> {
        let decoded = DecodedAudio::decode(input)?;
        let params = decoded.params()?;
        let chain = filters::build_chain(
            decoded.duration_secs(),
            &options.operations,
            options.custom_filters.as_deref(),
        )?;

        let graph = unsafe { FilterGraph::build(&params, &chain) }?;
        for raw in decoded.as_raw_frames() {
            unsafe { graph.push(raw.cast())? };
        }
        unsafe { graph.close()? };

        let mut sink_frames: Vec<*mut c_void> = Vec::new();
        while let Some(frame) = graph.pull()? {
            sink_frames.push(frame.into_raw().cast());
        }
        let processed = unsafe { DecodedAudio::from_raw_frames(sink_frames) }?;

        let format = options.format.unwrap_or(AudioFormat::Wav);
        let quality = options.quality.unwrap_or(80);
        processed.encode(format, quality)
    }
}
