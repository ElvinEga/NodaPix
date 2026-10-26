//! NodaPix Imaging: Pure image-processing engine for macOS and Apple Silicon.

pub mod backend;
pub mod buffer;
pub mod color;
pub mod filter;
pub mod transform;

pub use backend::{ExportFormat, ExportOptions, ImageBackend, ImageError, RustImageBackend};
pub use buffer::{
    linear_to_srgb, linear_to_srgb_u8, srgb_to_linear, srgb_u8_to_linear, PixelBuffer,
};
pub use color::{apply_color_adjustments, lerp, midtone_weight, ColorAdjustments};
pub use filter::{gaussian_blur, unsharp_mask};
pub use transform::{
    crop, downsample_to_fit, flip_horizontal, flip_vertical, rotate_orthogonal, CropRect,
    OrthogonalRotation,
};
