#![no_main]

use libfuzzer_sys::fuzz_target;
use image_conversion::{ConversionOptions, ImageConverter, ImageFormat};

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

    let options = ConversionOptions {
        format,
        quality,
        width,
        height,
    };

    if let Ok(converter) = ImageConverter::new() {
        let _ = converter.convert(payload, options);
    }
});
