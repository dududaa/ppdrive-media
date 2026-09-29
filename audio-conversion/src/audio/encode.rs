use crate::audio::options::{AudioFormat, bitrate_for_quality};
use crate::error::Error;
use crate::ffi;
use crate::ffi::encode::{
    EncodeConfig, EncodeOutput, EncodedPacket, Encoder, EncoderParams, InputParams,
};
use crate::ffi::wrappers::{Frame, find_encoder_by_id, find_encoder_by_name};

const MP3_RATES: &[u32] = &[8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000];
const AAC_RATES: &[u32] = &[
    7350, 8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000,
];

pub(crate) struct EncodeSpec {
    pub muxer: &'static str,
    pub filename: &'static str,
    pub codec_id: ffi::AVCodecID,
    pub codec_name: Option<&'static str>,
    pub sample_fmt: ffi::AVSampleFormat,
    pub fixed_rate: Option<u32>,
    pub rates: Option<&'static [u32]>,
    pub max_channels: u8,
}

pub(crate) fn spec_for(format: AudioFormat) -> EncodeSpec {
    match format {
        AudioFormat::Wav => EncodeSpec {
            muxer: "wav",
            filename: "out.wav",
            codec_id: ffi::AV_CODEC_ID_PCM_S16LE,
            codec_name: None,
            sample_fmt: ffi::AV_SAMPLE_FMT_S16,
            fixed_rate: None,
            rates: None,
            max_channels: 2,
        },
        AudioFormat::Mp3 => EncodeSpec {
            muxer: "mp3",
            filename: "out.mp3",
            codec_id: ffi::AV_CODEC_ID_MP3,
            codec_name: Some("libmp3lame"),
            sample_fmt: ffi::AV_SAMPLE_FMT_S16P,
            fixed_rate: None,
            rates: Some(MP3_RATES),
            max_channels: 2,
        },
        AudioFormat::Flac => EncodeSpec {
            muxer: "flac",
            filename: "out.flac",
            codec_id: ffi::AV_CODEC_ID_FLAC,
            codec_name: None,
            sample_fmt: ffi::AV_SAMPLE_FMT_S16,
            fixed_rate: None,
            rates: None,
            max_channels: 2,
        },
        AudioFormat::Aac => EncodeSpec {
            muxer: "adts",
            filename: "out.aac",
            codec_id: ffi::AV_CODEC_ID_AAC,
            codec_name: None,
            sample_fmt: ffi::AV_SAMPLE_FMT_FLTP,
            fixed_rate: None,
            rates: Some(AAC_RATES),
            max_channels: 2,
        },
        AudioFormat::Ogg => EncodeSpec {
            muxer: "ogg",
            filename: "out.ogg",
            codec_id: ffi::AV_CODEC_ID_VORBIS,
            codec_name: Some("libvorbis"),
            sample_fmt: ffi::AV_SAMPLE_FMT_FLTP,
            fixed_rate: None,
            rates: None,
            max_channels: 2,
        },
        AudioFormat::Opus => EncodeSpec {
            muxer: "opus",
            filename: "out.opus",
            codec_id: ffi::AV_CODEC_ID_OPUS,
            codec_name: Some("libopus"),
            sample_fmt: ffi::AV_SAMPLE_FMT_S16,
            fixed_rate: Some(48000),
            rates: None,
            max_channels: 2,
        },
    }
}

pub(crate) fn resolve_rate(
    spec: &EncodeSpec,
    requested: Option<u32>,
    input_rate: u32,
) -> Result<u32, Error> {
    if let Some(rate) = requested
        && rate == 0
    {
        return Err(Error::InvalidInput);
    }
    let requested = requested.unwrap_or(input_rate);
    if requested == 0 {
        return Err(Error::InvalidInput);
    }
    Ok(match spec.fixed_rate {
        Some(fixed) => fixed,
        None => match spec.rates {
            None => requested,
            Some(rates) => *rates
                .iter()
                .min_by_key(|&&rate| rate.abs_diff(requested))
                .unwrap_or(&requested),
        },
    })
}

pub(crate) fn resolve_channels(
    spec: &EncodeSpec,
    requested: Option<u8>,
    input_channels: u8,
) -> Result<u8, Error> {
    match requested {
        Some(channels) => {
            if channels == 0 || channels > spec.max_channels {
                return Err(Error::InvalidInput);
            }
            Ok(channels)
        }
        None => Ok(input_channels.clamp(1, spec.max_channels)),
    }
}

fn open_encoder(
    spec: &EncodeSpec,
    format: AudioFormat,
    quality: u8,
    out_rate: u32,
    out_channels: u8,
    input: &InputParams,
    output: EncodeOutput,
) -> Result<Encoder, Error> {
    let codec = match spec.codec_name {
        Some(name) => find_encoder_by_name(name).or_else(|_| find_encoder_by_id(spec.codec_id))?,
        None => find_encoder_by_id(spec.codec_id)?,
    };
    Encoder::new(
        EncodeConfig {
            codec,
            muxer: spec.muxer.to_string(),
            filename: spec.filename.to_string(),
            sample_fmt: spec.sample_fmt,
            sample_rate: out_rate,
            channels: out_channels,
            bit_rate: bitrate_for_quality(format, quality).map(u64::from),
            options: Vec::new(),
            output,
        },
        input,
    )
}

fn prepare(
    frames: &[Frame],
    input: &InputParams,
    format: AudioFormat,
    sample_rate: Option<u32>,
    channels: Option<u8>,
) -> Result<(EncodeSpec, u32, u8), Error> {
    if frames.is_empty() {
        return Err(Error::InvalidInput);
    }
    let spec = spec_for(format);
    let out_rate = resolve_rate(&spec, sample_rate, input.sample_rate)?;
    let out_channels = resolve_channels(&spec, channels, input.channels)?;
    Ok((spec, out_rate, out_channels))
}

pub(crate) fn encode_muxed(
    frames: &[Frame],
    input: &InputParams,
    format: AudioFormat,
    quality: u8,
    sample_rate: Option<u32>,
    channels: Option<u8>,
) -> Result<Vec<u8>, Error> {
    let (spec, out_rate, out_channels) = prepare(frames, input, format, sample_rate, channels)?;
    let mut encoder = open_encoder(
        &spec,
        format,
        quality,
        out_rate,
        out_channels,
        input,
        EncodeOutput::Muxed,
    )?;
    for frame in frames {
        encoder.encode_frame(frame)?;
    }
    encoder.finish()
}

pub(crate) fn encode_packetized<F>(
    frames: &[Frame],
    input: &InputParams,
    format: AudioFormat,
    quality: u8,
    global_headers: bool,
    sink: &mut F,
) -> Result<(), Error>
where
    F: FnMut(&EncoderParams, &EncodedPacket),
{
    let (spec, out_rate, out_channels) = prepare(frames, input, format, None, None)?;
    let mut encoder = open_encoder(
        &spec,
        format,
        quality,
        out_rate,
        out_channels,
        input,
        EncodeOutput::Packets { global_headers },
    )?;
    for frame in frames {
        encoder.encode_frame(frame)?;
        let pending = encoder.take_pending();
        for packet in &pending {
            sink(encoder.params(), packet);
        }
    }
    encoder.finish_drain()?;
    let pending = encoder.take_pending();
    for packet in &pending {
        sink(encoder.params(), packet);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(format: AudioFormat) -> EncodeSpec {
        spec_for(format)
    }

    #[test]
    fn spec_metadata_is_consistent() {
        let mp3 = spec(AudioFormat::Mp3);
        assert_eq!(mp3.muxer, "mp3");
        assert_eq!(mp3.codec_name, Some("libmp3lame"));
        assert_eq!(mp3.sample_fmt, ffi::AV_SAMPLE_FMT_S16P);

        let aac = spec(AudioFormat::Aac);
        assert_eq!(aac.muxer, "adts");
        assert_eq!(aac.codec_name, None);
        assert_eq!(aac.sample_fmt, ffi::AV_SAMPLE_FMT_FLTP);

        let opus = spec(AudioFormat::Opus);
        assert_eq!(opus.fixed_rate, Some(48000));

        for format in [
            AudioFormat::Wav,
            AudioFormat::Mp3,
            AudioFormat::Flac,
            AudioFormat::Aac,
            AudioFormat::Ogg,
            AudioFormat::Opus,
        ] {
            assert_eq!(spec(format).max_channels, 2);
        }
    }

    #[test]
    fn rate_resolution_honours_tables() {
        let mp3 = spec(AudioFormat::Mp3);
        assert_eq!(resolve_rate(&mp3, None, 44100).unwrap(), 44100);
        assert_eq!(resolve_rate(&mp3, None, 64000).unwrap(), 48000);
        assert_eq!(
            resolve_rate(&mp3, Some(0), 44100).unwrap_err(),
            Error::InvalidInput
        );
        assert_eq!(resolve_rate(&mp3, Some(32000), 44100).unwrap(), 32000);

        let opus = spec(AudioFormat::Opus);
        assert_eq!(resolve_rate(&opus, None, 44100).unwrap(), 48000);
        assert_eq!(resolve_rate(&opus, Some(8000), 48000).unwrap(), 48000);

        let wav = spec(AudioFormat::Wav);
        assert_eq!(resolve_rate(&wav, None, 96000).unwrap(), 96000);
        assert_eq!(resolve_rate(&wav, Some(22050), 44100).unwrap(), 22050);
    }

    #[test]
    fn channel_resolution_validates() {
        let spec = spec(AudioFormat::Mp3);
        assert_eq!(resolve_channels(&spec, None, 2).unwrap(), 2);
        assert_eq!(resolve_channels(&spec, None, 6).unwrap(), 2);
        assert_eq!(resolve_channels(&spec, Some(1), 2).unwrap(), 1);
        assert_eq!(
            resolve_channels(&spec, Some(0), 2).unwrap_err(),
            Error::InvalidInput
        );
        assert_eq!(
            resolve_channels(&spec, Some(6), 2).unwrap_err(),
            Error::InvalidInput
        );
    }
}
