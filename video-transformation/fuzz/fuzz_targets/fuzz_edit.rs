#![no_main]

use libfuzzer_sys::fuzz_target;
use video_transformation::{TransformOperation, TransformOptions, VideoFormat, VideoTransformer};

fuzz_target!(|data: &[u8]| {
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };
    let unit = u32::from(selector);

    if selector % 14 == 13 {
        if let Ok(options) = serde_json::from_slice::<TransformOptions>(payload) {
            if let Ok(transformer) = VideoTransformer::new() {
                let _ = transformer.transform(payload, options);
            }
        }
        return;
    }

    let operations = match selector % 14 {
        0 => vec![TransformOperation::Crop {
            x: unit,
            y: unit / 2,
            width: unit + 1,
            height: unit + 1,
        }],
        1 => vec![TransformOperation::Rotate {
            degrees: [90, 180, 270][usize::from(selector) % 3],
        }],
        2 => vec![TransformOperation::Flip {
            horizontal: selector & 1 != 0,
            vertical: selector & 2 != 0,
        }],
        3 => vec![TransformOperation::Pad {
            left: unit % 32,
            top: unit % 32,
            right: unit % 32,
            bottom: unit % 32,
            color: format!("#{selector:02x}{selector:02x}{selector:02x}"),
        }],
        4 => vec![TransformOperation::Grayscale],
        5 => vec![TransformOperation::Adjust {
            brightness: (f32::from(selector) - 127.5) / 127.5,
            contrast: f32::from(selector) / 64.0,
            saturation: f32::from(selector) / 64.0,
        }],
        6 => vec![TransformOperation::Blur {
            sigma: f32::from(selector % 20) / 4.0,
        }],
        7 => vec![TransformOperation::Sharpen {
            amount: f32::from(selector % 30) / 6.0,
        }],
        8 => vec![TransformOperation::Scale {
            width: unit % 64 + 1,
            height: unit % 64 + 1,
        }],
        9 => vec![TransformOperation::Trim {
            start: f64::from(unit % 100) / 10.0,
            duration: (selector % 2 == 0)
                .then(|| f64::from(unit % 50) / 10.0),
        }],
        10 => vec![TransformOperation::Speed {
            factor: f64::from(unit % 40 + 1) / 4.0,
        }],
        11 => vec![TransformOperation::Reverse],
        12 => Vec::new(),
        _ => unreachable!(),
    };

    let format = if selector % 2 == 0 {
        Some(VideoFormat::Mp4)
    } else {
        Some(VideoFormat::WebM)
    };

    let options = TransformOptions {
        operations,
        custom_filters: Some(String::from_utf8_lossy(payload).into_owned()),
        format,
        quality: Some(selector % 101),
    };

    if let Ok(transformer) = VideoTransformer::new() {
        let _ = transformer.transform(payload, options);
    }
});
