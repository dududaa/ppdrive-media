#![no_main]

use libfuzzer_sys::fuzz_target;
use ppff_video_conversion::{ConversionOptions, VideoConverter, VideoFormat};

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
    let scale = (selector & 0x20 != 0).then(|| f32::from(selector % 40) / 10.0 + 0.1);
    let effort = (selector & 0x10 != 0).then(|| selector % 101);
    let fps = (selector & 0x08 != 0).then(|| u32::from(selector % 30) + 1);
    let drop_audio = selector & 0x04 != 0;
    let max_bytes = (selector & 0x02 != 0)
        .then(|| payload.first().copied())
        .flatten()
        .map(|b| u64::from(b) * 1024);
    let keyframe_interval = (selector & 0x01 != 0).then(|| u32::from(selector % 60) + 1);

    let options = ConversionOptions {
        format,
        quality,
        width,
        height,
        scale,
        effort,
        max_bytes,
        fps,
        drop_audio,
        keyframe_interval,
    };

    if let Ok(converter) = VideoConverter::new() {
        let _ = converter.convert(payload, options);
    }
});
