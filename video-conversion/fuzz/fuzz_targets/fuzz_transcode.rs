#![no_main]

use libfuzzer_sys::fuzz_target;
use video_conversion::{ConversionOptions, VideoConverter, VideoFormat};

fuzz_target!(|data: &[u8]| {
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };

    let format = [
        VideoFormat::Mp4,
        VideoFormat::WebM,
        VideoFormat::Mov,
        VideoFormat::Mkv,
        VideoFormat::Avi,
        VideoFormat::Mp4Av1,
        VideoFormat::WebMAv1,
        VideoFormat::MkvAv1,
        VideoFormat::Mp4Hevc,
        VideoFormat::MovHevc,
    ][usize::from(selector % 10)];
    let quality = selector % 101;
    let width = (selector & 0x80 != 0).then(|| u32::from(selector % 64) + 1);
    let height = (selector & 0x40 != 0).then(|| u32::from(selector % 64) + 1);

    let options = ConversionOptions {
        format,
        quality,
        width,
        height,
    };

    if let Ok(converter) = VideoConverter::new() {
        let _ = converter.convert(payload, options);
    }
});
