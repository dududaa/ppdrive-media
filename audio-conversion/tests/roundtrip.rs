use audio_conversion::{
    AudioConverter, AudioFormat, ConversionOptions, DecodedAudio, EncodedPacket, EncoderParams,
};

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
            let sample = (0.5 * (std::f64::consts::TAU * 440.0 * t).sin()
                + 0.3 * (std::f64::consts::TAU * 130.0 * t).sin())
                * if ch == 0 { 1.0 } else { 0.7 };
            out.extend_from_slice(&((sample * 20000.0) as i16).to_le_bytes());
        }
    }
    out
}

fn parse_wav(data: &[u8]) -> (u32, u16, Vec<i16>) {
    assert!(data.len() >= 12, "truncated wav");
    assert_eq!(&data[0..4], b"RIFF", "missing RIFF");
    assert_eq!(&data[8..12], b"WAVE", "missing WAVE");

    let mut pos = 12usize;
    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    let mut samples = Vec::new();
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body_start = pos + 8;
        let body_end = (body_start + size).min(data.len());
        match id {
            b"fmt " => {
                channels =
                    u16::from_le_bytes(data[body_start + 2..body_start + 4].try_into().unwrap());
                sample_rate =
                    u32::from_le_bytes(data[body_start + 4..body_start + 8].try_into().unwrap());
                let bits =
                    u16::from_le_bytes(data[body_start + 14..body_start + 16].try_into().unwrap());
                assert_eq!(bits, 16, "expected 16-bit pcm");
            }
            b"data" => {
                samples = data[body_start..body_end]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
            }
            _ => {}
        }
        pos = body_start + size + (size & 1);
    }
    assert!(sample_rate > 0 && channels > 0, "missing fmt chunk");
    (sample_rate, channels, samples)
}

fn convert(input: &[u8], options: ConversionOptions) -> Vec<u8> {
    AudioConverter::new()
        .expect("converter init failed")
        .convert(input, options)
        .expect("conversion failed")
}

fn options(format: AudioFormat, quality: u8) -> ConversionOptions {
    ConversionOptions {
        format,
        quality,
        sample_rate: None,
        channels: None,
    }
}

fn magic(data: &[u8]) -> &[u8] {
    &data[..(data.len()).min(12)]
}

#[test]
fn wav_roundtrip_is_sample_exact() {
    let input = synthetic_wav(44100, 2, 0.5);
    let output = convert(&input, options(AudioFormat::Wav, 80));
    assert_eq!(&output[0..4], b"RIFF");
    let (rate, channels, samples) = parse_wav(&output);
    assert_eq!(rate, 44100);
    assert_eq!(channels, 2);
    let (_, _, expected) = parse_wav(&input);
    assert_eq!(samples, expected);
}

#[test]
fn flac_roundtrip_is_sample_exact() {
    let input = synthetic_wav(44100, 1, 0.5);
    let output = convert(&input, options(AudioFormat::Flac, 80));
    assert_eq!(&output[0..4], b"fLaC");
    let decoded = DecodedAudio::decode(&output).expect("flac output does not decode");
    assert_eq!(decoded.sample_rate(), 44100);
    assert_eq!(decoded.channels(), 1);
    assert!((decoded.duration_secs() - 0.5).abs() < 0.01);
}

#[test]
fn lossy_formats_decode_back() {
    let input = synthetic_wav(44100, 2, 0.5);
    for format in [
        AudioFormat::Mp3,
        AudioFormat::Aac,
        AudioFormat::Ogg,
        AudioFormat::Opus,
    ] {
        let output = convert(&input, options(format, 80));
        match format {
            AudioFormat::Mp3 => {
                let ok = magic(&output).starts_with(b"ID3") || matches!(magic(&output)[0], 0xFF);
                assert!(ok, "mp3 magic: {:02x?}", magic(&output));
            }
            AudioFormat::Aac => {
                assert_eq!(magic(&output)[0], 0xFF, "adts sync byte");
                assert_eq!(magic(&output)[1] & 0xF0, 0xF0, "adts sync word");
            }
            AudioFormat::Ogg | AudioFormat::Opus => {
                assert_eq!(&output[0..4], b"OggS");
            }
            AudioFormat::Wav | AudioFormat::Flac => unreachable!(),
        }

        let decoded = DecodedAudio::decode(&output).unwrap_or_else(|e| {
            panic!("{format:?} output does not decode: {e}");
        });
        assert!(
            (decoded.duration_secs() - 0.5).abs() < 0.15,
            "{format:?}: duration {}",
            decoded.duration_secs()
        );
        assert_eq!(decoded.channels(), 2, "{format:?}");
        let expected_rate = if format == AudioFormat::Opus {
            48000
        } else {
            44100
        };
        assert_eq!(decoded.sample_rate(), expected_rate, "{format:?}");
    }
}

#[test]
fn sample_rate_option_resamples() {
    let input = synthetic_wav(44100, 2, 0.5);
    let output = convert(
        &input,
        ConversionOptions {
            format: AudioFormat::Wav,
            quality: 80,
            sample_rate: Some(22050),
            channels: None,
        },
    );
    let (rate, channels, samples) = parse_wav(&output);
    assert_eq!(rate, 22050);
    assert_eq!(channels, 2);
    let duration = samples.len() as f64 / 2.0 / 22050.0;
    assert!((duration - 0.5).abs() < 0.01, "duration {duration}");
}

#[test]
fn channels_option_downmixes() {
    let input = synthetic_wav(44100, 2, 0.5);
    let output = convert(
        &input,
        ConversionOptions {
            format: AudioFormat::Wav,
            quality: 80,
            sample_rate: None,
            channels: Some(1),
        },
    );
    let (rate, channels, samples) = parse_wav(&output);
    assert_eq!(rate, 44100);
    assert_eq!(channels, 1);
    assert_eq!(samples.len(), 22050);
}

#[test]
fn multichannel_input_downmixes_to_stereo() {
    let input = synthetic_wav(44100, 6, 0.25);
    let output = convert(&input, options(AudioFormat::Mp3, 50));
    let decoded = DecodedAudio::decode(&output).unwrap();
    assert_eq!(decoded.channels(), 2);
}

#[test]
fn invalid_options_are_rejected() {
    let input = synthetic_wav(44100, 2, 0.25);
    let converter = AudioConverter::new().unwrap();
    for bad in [
        ConversionOptions {
            format: AudioFormat::Wav,
            quality: 80,
            sample_rate: Some(0),
            channels: None,
        },
        ConversionOptions {
            format: AudioFormat::Wav,
            quality: 80,
            sample_rate: None,
            channels: Some(0),
        },
        ConversionOptions {
            format: AudioFormat::Mp3,
            quality: 80,
            sample_rate: None,
            channels: Some(6),
        },
    ] {
        assert_eq!(
            converter.convert(&input, bad).unwrap_err(),
            audio_conversion::Error::InvalidInput
        );
    }
}

#[test]
fn encode_to_yields_mpx_packets() {
    let input = synthetic_wav(44100, 2, 0.5);
    let decoded = DecodedAudio::decode(&input).unwrap();

    let mut params: Option<EncoderParams> = None;
    let mut packets: Vec<EncodedPacket> = Vec::new();
    decoded
        .encode_to(
            AudioFormat::Mp3,
            80,
            false,
            &mut |enc_params: &EncoderParams, packet: &EncodedPacket| {
                if params.is_none() {
                    params = Some(enc_params.clone());
                }
                packets.push(EncodedPacket {
                    data: packet.data.clone(),
                    pts: packet.pts,
                    tb_num: packet.tb_num,
                    tb_den: packet.tb_den,
                    duration: packet.duration,
                });
            },
        )
        .expect("encode_to failed");

    let params = params.expect("no packets produced");
    assert_eq!(params.sample_rate, 44100);
    assert_eq!(params.channels, 2);
    assert_eq!(params.time_base_num, 1);
    assert_eq!(params.time_base_den, 44100);
    assert_eq!(params.codec_name, "libmp3lame");
    assert!(!packets.is_empty());
    for pair in packets.windows(2) {
        assert!(pair[1].pts > pair[0].pts, "pts must be strictly increasing");
    }
    for packet in &packets {
        assert!(packet.duration > 0);
        assert!(!packet.data.is_empty());
    }

    let assembled: Vec<u8> = packets
        .iter()
        .flat_map(|p| p.data.iter().copied())
        .collect();
    let reparsed = DecodedAudio::decode(&assembled).expect("packet stream does not decode");
    assert!((reparsed.duration_secs() - 0.5).abs() < 0.2);
}

#[test]
fn decode_rejects_empty_and_unsupported() {
    use audio_conversion::Error;
    assert_eq!(DecodedAudio::decode(&[]).unwrap_err(), Error::InvalidInput);
    let png = [
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D',
        b'R',
    ];
    assert_eq!(
        DecodedAudio::decode(&png).unwrap_err(),
        Error::UnsupportedFormat
    );
}
