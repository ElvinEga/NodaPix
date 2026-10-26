//! ImageBackend abstraction and default Rust `image` crate codec.

use crate::buffer::PixelBuffer;
use crate::metadata::ImageMetadata;
use crate::transform::apply_exif_orientation;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, BufWriter, Cursor};
use std::path::Path;
use thiserror::Error;

/// Decoders hand back the stored raster, so rotate it into display orientation.
pub(crate) fn orient(decoded: PixelBuffer<f32>, bytes: &[u8]) -> PixelBuffer<f32> {
    let block = match ImageMetadata::read_exif_block(bytes) {
        Some(block) => block,
        None => return decoded,
    };
    match ImageMetadata::orientation(&block) {
        1 => decoded,
        other => apply_exif_orientation(&decoded, other),
    }
}

#[derive(Error, Debug)]
pub enum ImageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Codec error: {0}")]
    Codec(#[from] image::ImageError),
    #[error("Unsupported format: {0}")]
    Unsupported(String),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    Jpeg,
    Png,
    WebP,
    Tiff,
}

impl ExportFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::WebP => "webp",
            Self::Tiff => "tiff",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "webp" => Some(Self::WebP),
            "tif" | "tiff" => Some(Self::Tiff),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub quality: u8, // 1 to 100 for lossy formats (JPEG, WebP)
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Jpeg,
            quality: 90,
        }
    }
}

pub trait ImageBackend: Send + Sync {
    fn decode_from_bytes(&self, bytes: &[u8]) -> Result<PixelBuffer<f32>, ImageError>;
    fn decode_from_path(&self, path: &Path) -> Result<PixelBuffer<f32>, ImageError>;
    fn encode_to_bytes(
        &self,
        buffer: &PixelBuffer<f32>,
        options: &ExportOptions,
    ) -> Result<Vec<u8>, ImageError>;
    fn encode_to_path(
        &self,
        buffer: &PixelBuffer<f32>,
        path: &Path,
        options: &ExportOptions,
    ) -> Result<(), ImageError>;
}

/// Default pure-Rust codec implementation using `image` crate.
#[derive(Default, Clone, Debug)]
pub struct RustImageBackend;

impl RustImageBackend {
    pub fn new() -> Self {
        Self
    }
}

impl ImageBackend for RustImageBackend {
    fn decode_from_bytes(&self, bytes: &[u8]) -> Result<PixelBuffer<f32>, ImageError> {
        let dyn_img = image::load_from_memory(bytes)?;
        Ok(orient(PixelBuffer::from_dynamic_image(&dyn_img), bytes))
    }

    fn decode_from_path(&self, path: &Path) -> Result<PixelBuffer<f32>, ImageError> {
        let reader = BufReader::new(File::open(path)?);
        let dyn_img = image::load(reader, image::ImageFormat::from_path(path)?)?;
        let decoded = PixelBuffer::from_dynamic_image(&dyn_img);
        Ok(match ImageMetadata::read_exif_block_from_path(path) {
            Some(block) => match ImageMetadata::orientation(&block) {
                1 => decoded,
                other => apply_exif_orientation(&decoded, other),
            },
            None => decoded,
        })
    }

    fn encode_to_bytes(
        &self,
        buffer: &PixelBuffer<f32>,
        options: &ExportOptions,
    ) -> Result<Vec<u8>, ImageError> {
        let rgba = buffer.to_rgba_image();
        let mut cursor = Cursor::new(Vec::new());

        match options.format {
            ExportFormat::Png => {
                rgba.write_to(&mut cursor, image::ImageFormat::Png)?;
            }
            ExportFormat::Jpeg => {
                let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
                let mut encoder =
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, options.quality);
                encoder.encode_image(&rgb)?;
            }
            ExportFormat::WebP => {
                rgba.write_to(&mut cursor, image::ImageFormat::WebP)?;
            }
            ExportFormat::Tiff => {
                rgba.write_to(&mut cursor, image::ImageFormat::Tiff)?;
            }
        }

        Ok(cursor.into_inner())
    }

    fn encode_to_path(
        &self,
        buffer: &PixelBuffer<f32>,
        path: &Path,
        options: &ExportOptions,
    ) -> Result<(), ImageError> {
        let bytes = self.encode_to_bytes(buffer, options)?;
        let mut writer = BufWriter::new(File::create(path)?);
        use std::io::Write;
        writer.write_all(&bytes)?;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub mod apple_image_io;

#[cfg(target_os = "macos")]
pub use apple_image_io::AppleImageIOBackend;

#[cfg(target_os = "macos")]
pub type NativeBackend = AppleImageIOBackend;

#[cfg(not(target_os = "macos"))]
pub type NativeBackend = RustImageBackend;

