//! NodaPix Imaging: Pure image-processing engine for macOS and Apple Silicon.

pub mod backend;
pub mod buffer;
pub mod color;
pub mod filter;
pub mod metadata;
pub mod transform;

pub use backend::{
    ExportFormat, ExportOptions, ImageBackend, ImageError, NativeBackend, RustImageBackend,
};
#[cfg(target_os = "macos")]
pub use backend::AppleImageIOBackend;
pub use buffer::{
    linear_to_srgb, linear_to_srgb_u8, srgb_to_linear, srgb_u8_to_linear, PixelBuffer,
};
pub use color::{
    apply_color_adjustments, calculate_auto_levels, compute_histogram,
    kelvin_to_white_balance_gains, lerp, midtone_weight, slider_to_kelvin, ColorAdjustments,
    FilterPreset, ImageHistogram, TEMPERATURE_MAX_KELVIN, TEMPERATURE_MIN_KELVIN,
    TEMPERATURE_NEUTRAL_KELVIN,
};
pub use filter::{gaussian_blur, unsharp_mask};
pub use metadata::ImageMetadata;
pub use transform::{
    crop, downsample_to_fit, flip_horizontal, flip_vertical, resize, rotate_orthogonal,
    rotate_straighten_autocrop, straightened_size, CropRatioPreset, CropRect, OrthogonalRotation,
};
