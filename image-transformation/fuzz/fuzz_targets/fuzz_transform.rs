#![no_main]

use libfuzzer_sys::fuzz_target;
use image_transformation::{ImageTransformer, TransformOperation, TransformOptions};

fuzz_target!(|data: &[u8]| {
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };
    let unit = u32::from(selector);

    let operations = match selector % 9 {
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
        _ => vec![TransformOperation::Scale {
            width: unit % 64 + 1,
            height: unit % 64 + 1,
        }],
    };

    let options = TransformOptions {
        operations,
        custom_filters: Some(String::from_utf8_lossy(payload).into_owned()),
        format: None,
        quality: Some(selector % 101),
    };

    if let Ok(transformer) = ImageTransformer::new() {
        let _ = transformer.transform(payload, options);
    }
});
