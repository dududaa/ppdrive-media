#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    WebP,
    Avif,
}

impl ImageFormat {
    pub fn from_extension(ext: &str) -> Option<ImageFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
            "png" => Some(ImageFormat::Png),
            "webp" => Some(ImageFormat::WebP),
            "avif" => Some(ImageFormat::Avif),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressionOptions {
    pub format: ImageFormat,
    pub quality: u8,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl Default for CompressionOptions {
    fn default() -> Self {
        CompressionOptions {
            format: ImageFormat::Jpeg,
            quality: 80,
            width: None,
            height: None,
        }
    }
}
