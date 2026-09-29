#![no_main]

use libfuzzer_sys::fuzz_target;
use video_compression::{CompressionOptions, VideoCompressor, VideoFormat};

fuzz_target!(|data: &[u8]| {
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };

    let format = if selector % 2 == 0 {
        VideoFormat::Mp4
    } else {
        VideoFormat::WebM
    };
    let quality = selector % 101;
    let width = (selector & 0x80 != 0).then(|| u32::from(selector % 64) + 1);
    let height = (selector & 0x40 != 0).then(|| u32::from(selector % 64) + 1);

    let options = CompressionOptions {
        format,
        quality,
        width,
        height,
    };

    if let Ok(compressor) = VideoCompressor::new() {
        let _ = compressor.compress(payload, options);
    }
});
