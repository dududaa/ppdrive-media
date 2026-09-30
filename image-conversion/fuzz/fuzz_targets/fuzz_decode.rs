#![no_main]

use image_conversion::{ConversionOptions, ImageConverter, ImageFormat};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };

    let format = match selector % 4 {
        0 => ImageFormat::Jpeg,
        1 => ImageFormat::Png,
        2 => ImageFormat::WebP,
        _ => ImageFormat::Avif,
    };
    let quality = selector % 101;
    let width = (selector & 0x80 != 0).then(|| u32::from(selector % 64) + 1);
    let height = (selector & 0x40 != 0).then(|| u32::from(selector % 64) + 1);
    let scale = (selector & 0x20 != 0).then(|| f32::from(selector % 40 + 1) / 10.0);
    let effort = (selector & 0x10 != 0).then(|| selector % 101);
    let max_bytes = payload.first().map(|&b| u64::from(b) * 1024);

    let options = ConversionOptions {
        format,
        quality,
        width,
        height,
        scale,
        effort,
        max_bytes,
    };

    if let Ok(converter) = ImageConverter::new() {
        let _ = converter.convert(payload, options);
    }
});
