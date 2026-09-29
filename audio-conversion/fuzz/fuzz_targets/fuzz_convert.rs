#![no_main]

use libfuzzer_sys::fuzz_target;
use audio_conversion::{AudioConverter, AudioFormat, ConversionOptions};

fuzz_target!(|data: &[u8]| {
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };

    let format = match selector % 6 {
        0 => AudioFormat::Wav,
        1 => AudioFormat::Mp3,
        2 => AudioFormat::Flac,
        3 => AudioFormat::Aac,
        4 => AudioFormat::Ogg,
        _ => AudioFormat::Opus,
    };
    let quality = selector % 101;
    let sample_rate = (selector & 0x80 != 0).then(|| u32::from(selector % 8 + 1) * 8000);
    let channels = (selector & 0x40 != 0).then(|| selector % 2 + 1);

    let options = ConversionOptions {
        format,
        quality,
        sample_rate,
        channels,
    };

    if let Ok(converter) = AudioConverter::new() {
        let _ = converter.convert(payload, options);
    }
});
