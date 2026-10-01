use ppff_audio_conversion::{AudioFormat, DecodedAudio, Error};
use audio_effects::{AudioEffects, EffectOperation, EffectOptions};

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

fn apply(input: &[u8], operations: Vec<EffectOperation>) -> Result<Vec<u8>, Error> {
    AudioEffects::new()?.apply(
        input,
        EffectOptions {
            operations,
            custom_filters: None,
            format: None,
            quality: None,
        },
    )
}

fn duration(data: &[u8]) -> f64 {
    let (rate, channels, samples) = parse_wav(data);
    samples.len() as f64 / f64::from(channels) / f64::from(rate)
}

fn peak(samples: &[i16]) -> u16 {
    samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0)
}

#[test]
fn identity_pass_is_sample_exact() {
    let input = synthetic_wav(44100, 2, 1.0);
    let output = apply(&input, vec![]).unwrap();
    assert_eq!(&output[0..4], b"RIFF");
    let (_, _, in_samples) = parse_wav(&input);
    let (_, _, out_samples) = parse_wav(&output);
    assert_eq!(out_samples, in_samples);
}

#[test]
fn volume_attenuates_peak_by_expected_ratio() {
    let input = synthetic_wav(44100, 2, 1.0);
    let output = apply(&input, vec![EffectOperation::Volume { gain_db: -6.0 }]).unwrap();
    let (_, _, in_samples) = parse_wav(&input);
    let (_, _, out_samples) = parse_wav(&output);
    let ratio = f64::from(peak(&out_samples)) / f64::from(peak(&in_samples));
    let expected = 10f64.powf(-6.0 / 20.0);
    assert!(
        (ratio - expected).abs() < expected * 0.05,
        "ratio {ratio}, expected {expected}"
    );
}

#[test]
fn speed_halves_duration() {
    let input = synthetic_wav(44100, 2, 1.0);
    let output = apply(&input, vec![EffectOperation::Speed { factor: 2.0 }]).unwrap();
    let dur = duration(&output);
    assert!((dur - 0.5).abs() < 0.1, "duration {dur}");
}

#[test]
fn trim_selects_window() {
    let input = synthetic_wav(44100, 2, 1.0);
    let output = apply(
        &input,
        vec![EffectOperation::Trim {
            start_secs: 0.25,
            end_secs: 0.75,
        }],
    )
    .unwrap();
    let dur = duration(&output);
    assert!((dur - 0.5).abs() < 0.05, "duration {dur}");
}

#[test]
fn reverse_flips_samples_exactly() {
    let input = synthetic_wav(44100, 1, 0.5);
    let output = apply(&input, vec![EffectOperation::Reverse]).unwrap();
    let (_, _, in_samples) = parse_wav(&input);
    let (_, _, out_samples) = parse_wav(&output);
    let mut expected = in_samples.clone();
    expected.reverse();
    assert_eq!(out_samples, expected);
}

#[test]
fn fade_silences_start_and_end() {
    let input = synthetic_wav(44100, 1, 1.0);
    let output = apply(
        &input,
        vec![EffectOperation::Fade {
            fade_in_secs: 0.2,
            fade_out_secs: 0.2,
        }],
    )
    .unwrap();
    let (_, _, out_samples) = parse_wav(&output);
    assert!(out_samples[0].unsigned_abs() <= 4, "start not faded");
    assert!(
        out_samples[out_samples.len() - 1].unsigned_abs() <= 4,
        "end not faded"
    );
    let mid_start = out_samples.len() * 4 / 10;
    let mid_end = out_samples.len() * 6 / 10;
    let middle_peak = peak(&out_samples[mid_start..mid_end]);
    assert!(middle_peak > 1000, "middle silenced: peak {middle_peak}");
}

#[test]
fn custom_filter_runs_last() {
    let input = synthetic_wav(44100, 2, 0.5);
    let output = AudioEffects::new()
        .unwrap()
        .apply(
            &input,
            EffectOptions {
                operations: vec![EffectOperation::Volume { gain_db: -6.0 }],
                custom_filters: Some("volume=0.5".to_string()),
                format: None,
                quality: None,
            },
        )
        .unwrap();
    let (_, _, in_samples) = parse_wav(&input);
    let (_, _, out_samples) = parse_wav(&output);
    let ratio = f64::from(peak(&out_samples)) / f64::from(peak(&in_samples));
    let expected = 10f64.powf(-6.0 / 20.0) * 0.5;
    assert!(
        (ratio - expected).abs() < expected * 0.08,
        "ratio {ratio}, expected {expected}"
    );
}

#[test]
fn eq_and_echo_preserve_duration() {
    let input = synthetic_wav(44100, 2, 1.0);
    let output = apply(
        &input,
        vec![
            EffectOperation::Bass {
                gain_db: 6.0,
                frequency: 100.0,
                width: 50.0,
            },
            EffectOperation::Treble {
                gain_db: -3.0,
                frequency: 3000.0,
                width: 1000.0,
            },
            EffectOperation::Echo {
                delay_ms: 120,
                decay: 0.4,
            },
        ],
    )
    .unwrap();
    let decoded_in = DecodedAudio::decode(&input).unwrap();
    let decoded_out = DecodedAudio::decode(&output).unwrap();
    let dur = decoded_out.duration_secs();
    assert!(
        dur >= decoded_in.duration_secs() - 0.01 && dur <= 1.15,
        "duration {dur}"
    );
    assert_eq!(decoded_out.channels(), 2);
}

#[test]
fn normalize_produces_valid_audio() {
    let input = synthetic_wav(44100, 2, 2.0);
    let output = apply(
        &input,
        vec![EffectOperation::Normalize { target_lufs: -16.0 }],
    )
    .unwrap();
    let decoded = DecodedAudio::decode(&output).unwrap();
    assert!(
        (decoded.duration_secs() - 2.0).abs() < 0.5,
        "duration {}",
        decoded.duration_secs()
    );
    assert_eq!(decoded.channels(), 2);
}

#[test]
fn mp3_output_option_encodes() {
    let input = synthetic_wav(44100, 2, 0.5);
    let output = AudioEffects::new()
        .unwrap()
        .apply(
            &input,
            EffectOptions {
                operations: vec![EffectOperation::Volume { gain_db: -3.0 }],
                custom_filters: None,
                format: Some(AudioFormat::Mp3),
                quality: Some(70),
            },
        )
        .unwrap();
    assert!(output.len() > 100);
    let decoded = DecodedAudio::decode(&output).unwrap();
    assert!((decoded.duration_secs() - 0.5).abs() < 0.15);
}

#[test]
fn invalid_operations_rejected_before_processing() {
    let input = synthetic_wav(44100, 2, 0.5);
    for bad in [
        vec![EffectOperation::Speed { factor: 0.0 }],
        vec![EffectOperation::Trim {
            start_secs: 10.0,
            end_secs: 11.0,
        }],
        vec![EffectOperation::Echo {
            delay_ms: 60_001,
            decay: 0.5,
        }],
        vec![EffectOperation::Volume { gain_db: f32::NAN }],
    ] {
        let err = apply(&input, bad).unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }
}

#[test]
fn malformed_input_rejected() {
    let effects = AudioEffects::new().unwrap();
    assert_eq!(
        effects.apply(&[], EffectOptions::default()).unwrap_err(),
        Error::InvalidInput
    );
    let png = [
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D',
        b'R',
    ];
    assert_eq!(
        effects.apply(&png, EffectOptions::default()).unwrap_err(),
        Error::UnsupportedFormat
    );
}

#[test]
fn decode_only_params_available_for_graph() {
    let input = synthetic_wav(48000, 1, 0.25);
    let decoded = DecodedAudio::decode(&input).unwrap();
    let params = decoded.params().unwrap();
    assert_eq!(params.sample_rate, 48000);
    assert_eq!(params.channels, 1);
    assert_eq!(params.channel_layout, "mono");
}
