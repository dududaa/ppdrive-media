#![no_main]

use audio_effects::{AudioEffects, EffectOperation, EffectOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };

    let operations = match selector % 10 {
        0 => vec![EffectOperation::Volume { gain_db: -6.0 }],
        1 => vec![EffectOperation::Fade {
            fade_in_secs: 0.1,
            fade_out_secs: 0.2,
        }],
        2 => vec![EffectOperation::Speed { factor: 2.0 }],
        3 => vec![EffectOperation::Trim {
            start_secs: 0.0,
            end_secs: 1.0,
        }],
        4 => vec![EffectOperation::Reverse],
        5 => vec![EffectOperation::Normalize {
            target_lufs: -16.0,
        }],
        6 => vec![EffectOperation::Echo {
            delay_ms: 250,
            decay: 0.5,
        }],
        7 => vec![EffectOperation::Bass {
            gain_db: 3.0,
            frequency: 100.0,
            width: 0.5,
        }],
        8 => vec![EffectOperation::Treble {
            gain_db: -3.0,
            frequency: 3000.0,
            width: 0.5,
        }],
        _ => vec![],
    };

    let options = EffectOptions {
        operations,
        custom_filters: None,
        format: None,
        quality: None,
    };

    if let Ok(effects) = AudioEffects::new() {
        let _ = effects.apply(payload, options);
    }
});
