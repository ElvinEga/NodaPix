//! NodaPix Imaging: Pure image-processing engine for macOS and Apple Silicon.

pub mod backend;
pub mod buffer;
pub mod color;
pub mod contact_sheet;
pub mod filter;
pub mod font;
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
    apply_color_adjustments, apply_red_eye_fix_to_buffer, calculate_auto_levels,
    calculate_white_balance_from_sample, compute_histogram, kelvin_to_white_balance_gains, lerp,
    midtone_weight, slider_to_kelvin, ColorAdjustments, CurvePoint, Curves, FilterPreset,
    ImageHistogram, RedEyeFix, ToneCurve, TEMPERATURE_MAX_KELVIN, TEMPERATURE_MIN_KELVIN,
    TEMPERATURE_NEUTRAL_KELVIN,
};
pub use contact_sheet::{
    generate_and_save_contact_sheets, generate_contact_sheet_page, ContactSheetConfig,
    SheetBackground, SheetPreset,
};
pub use filter::{gaussian_blur, unsharp_mask};
pub use font::{render_char, render_text, text_dimensions, truncate_to_width, FONT_8X8};
pub use metadata::{ImageMetadata, attach_exif};
pub use transform::{
    apply_exif_orientation, crop, downsample_to_fit, flip_horizontal, flip_vertical, resize,
    rotate_orthogonal, rotate_straighten_autocrop, straightened_size, CropRatioPreset, CropRect,
    OrthogonalRotation,
};
