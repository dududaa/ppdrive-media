use crate::options::{RenditionSpec, StreamingOptions};

/// A resolved rendition: concrete target dimensions, quality and the
/// bitrate advertised in the playlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rung {
    pub width: u32,
    pub height: u32,
    pub quality: u8,
    pub bitrate: u64,
}

/// Resolves the rendition ladder: the explicit
/// [`StreamingOptions::renditions`] list, or an auto ladder derived
/// from the source resolution.
pub(crate) fn resolve(
    options: &StreamingOptions,
    source_width: u32,
    source_height: u32,
    fps: f64,
) -> Vec<Rung> {
    match &options.renditions {
        Some(specs) => manual(specs, source_width, source_height, fps, options.quality),
        None => auto(source_width, source_height, fps, options.quality),
    }
}

/// Auto ladder: the source height (when below 1080p) plus every
/// standard rung at or below it, highest first. Never upscales.
fn auto(source_width: u32, source_height: u32, fps: f64, quality: u8) -> Vec<Rung> {
    let mut heights: Vec<u32> = Vec::new();
    if source_height < 1080 {
        heights.push(source_height);
    }
    for height in [1080u32, 720, 480, 360, 240] {
        if height <= source_height && !heights.contains(&height) {
            heights.push(height);
        }
    }
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();

    heights
        .into_iter()
        .map(|height| {
            let scale = f64::from(height) / f64::from(source_height.max(1));
            let width = even_up(scaled_dimension(source_width, scale));
            let height = even_up(height);
            Rung {
                width,
                height,
                quality,
                bitrate: bitrate_for(width, height, fps),
            }
        })
        .collect()
}

/// Manual ladder: each spec resolved to concrete dimensions with the
/// video-conversion semantics (explicit dims win, `scale` applies to
/// the source, aspect ratio derived when only one dimension is set).
fn manual(
    specs: &[RenditionSpec],
    source_width: u32,
    source_height: u32,
    fps: f64,
    base_quality: u8,
) -> Vec<Rung> {
    specs
        .iter()
        .map(|spec| {
            let (width, height) = resolve_dims(spec, source_width, source_height);
            Rung {
                width,
                height,
                quality: spec.quality.unwrap_or(base_quality).min(100),
                bitrate: spec
                    .bitrate
                    .unwrap_or_else(|| bitrate_for(width, height, fps)),
            }
        })
        .collect()
}

fn resolve_dims(spec: &RenditionSpec, source_width: u32, source_height: u32) -> (u32, u32) {
    match (spec.width, spec.height) {
        (Some(width), Some(height)) => (even_up(width), even_up(height)),
        (Some(width), None) => {
            let height = derived(width as f64 * f64::from(source_height) / f64::from(source_width));
            (even_up(width), even_up(height))
        }
        (None, Some(height)) => {
            let width = derived(height as f64 * f64::from(source_width) / f64::from(source_height));
            (even_up(width), even_up(height))
        }
        (None, None) => match spec.scale {
            Some(scale) => (
                even_up(scaled_dimension(source_width, f64::from(scale))),
                even_up(scaled_dimension(source_height, f64::from(scale))),
            ),
            None => (even_up(source_width), even_up(source_height)),
        },
    }
}

fn scaled_dimension(source: u32, scale: f64) -> u32 {
    derived(f64::from(source) * scale)
}

fn derived(value: f64) -> u32 {
    if !value.is_finite() || value <= 0.0 {
        return 1;
    }
    let rounded = value.round();
    if rounded >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        rounded.max(1.0) as u32
    }
}

/// Rounds up to an even value (chroma-subsampling requirement),
/// keeping at least 2×2.
fn even_up(value: u32) -> u32 {
    let value = value.max(1);
    if value % 2 == 1 {
        value.saturating_add(1)
    } else {
        value
    }
}

/// Advertised bitrate for a rung: a nominal ladder for standard
/// heights, an area×rate estimate for everything smaller.
pub(crate) fn bitrate_for(width: u32, height: u32, fps: f64) -> u64 {
    match height {
        ..=239 => estimate(width, height, fps),
        240..=359 => 400_000,
        360..=479 => 800_000,
        480..=719 => 1_400_000,
        720..=1079 => 2_800_000,
        _ => 5_000_000,
    }
}

fn estimate(width: u32, height: u32, fps: f64) -> u64 {
    let fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        25.0
    };
    let bits_per_second = f64::from(width) * f64::from(height) * fps * 0.07;
    let rounded = (bits_per_second / 50_000.0).round() * 50_000.0;
    rounded.max(100_000.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(spec: RenditionSpec) -> StreamingOptions {
        StreamingOptions {
            renditions: Some(vec![spec]),
            ..StreamingOptions::default()
        }
    }

    #[test]
    fn auto_ladder_from_1080p_covers_standard_rungs() {
        let rungs = auto(1920, 1080, 30.0, 80);
        let heights: Vec<u32> = rungs.iter().map(|r| r.height).collect();
        assert_eq!(heights, vec![1080, 720, 480, 360, 240]);
        assert_eq!(rungs[0].width, 1920);
        assert_eq!(rungs[0].bitrate, 5_000_000);
        assert_eq!(rungs[1].bitrate, 2_800_000);
        assert!(rungs.iter().all(|r| r.width % 2 == 0 && r.height % 2 == 0));
    }

    #[test]
    fn auto_ladder_never_upscales_small_sources() {
        let rungs = auto(320, 180, 25.0, 80);
        assert_eq!(rungs.len(), 1);
        assert_eq!(rungs[0].width, 320);
        assert_eq!(rungs[0].height, 180);

        let rungs = auto(640, 300, 25.0, 80);
        let heights: Vec<u32> = rungs.iter().map(|r| r.height).collect();
        assert_eq!(heights, vec![300, 240]);
        assert_eq!(rungs[0].width, 640);
    }

    #[test]
    fn auto_ladder_keeps_source_above_1080_out() {
        let rungs = auto(3840, 2160, 30.0, 80);
        let heights: Vec<u32> = rungs.iter().map(|r| r.height).collect();
        assert_eq!(heights, vec![1080, 720, 480, 360, 240]);
        assert_eq!(rungs[0].width, 1920);
    }

    #[test]
    fn manual_spec_resolves_dimensions_and_upscales() {
        let rungs = resolve(
            &spec(RenditionSpec {
                scale: Some(2.0),
                ..RenditionSpec::default()
            }),
            320,
            180,
            25.0,
        );
        assert_eq!(rungs.len(), 1);
        assert_eq!((rungs[0].width, rungs[0].height), (640, 360));
        assert_eq!(rungs[0].bitrate, 800_000);
        assert_eq!(rungs[0].quality, 80);
    }

    #[test]
    fn manual_spec_derives_missing_dimension() {
        let rungs = resolve(
            &spec(RenditionSpec {
                width: Some(160),
                ..RenditionSpec::default()
            }),
            320,
            180,
            25.0,
        );
        assert_eq!((rungs[0].width, rungs[0].height), (160, 90));

        let rungs = resolve(
            &spec(RenditionSpec {
                height: Some(90),
                quality: Some(55),
                bitrate: Some(123_456),
                ..RenditionSpec::default()
            }),
            320,
            180,
            25.0,
        );
        assert_eq!((rungs[0].width, rungs[0].height), (160, 90));
        assert_eq!(rungs[0].quality, 55);
        assert_eq!(rungs[0].bitrate, 123_456);
    }

    #[test]
    fn bitrate_ladder_steps_with_height() {
        assert_eq!(bitrate_for(1920, 1080, 30.0), 5_000_000);
        assert_eq!(bitrate_for(1280, 720, 30.0), 2_800_000);
        assert_eq!(bitrate_for(854, 480, 30.0), 1_400_000);
        assert_eq!(bitrate_for(640, 360, 30.0), 800_000);
        assert_eq!(bitrate_for(426, 240, 30.0), 400_000);
        let tiny = bitrate_for(160, 90, 25.0);
        assert!(tiny >= 100_000, "{tiny}");
    }
}
