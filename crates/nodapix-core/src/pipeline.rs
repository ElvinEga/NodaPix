//! Canonical non-destructive image rendering pipeline.

use crate::edit_state::EditState;
use nodapix_imaging::{
    apply_color_adjustments, crop, flip_horizontal, flip_vertical, rotate_orthogonal,
    unsharp_mask, PixelBuffer,
};

/// Execute the canonical editing pipeline against a source buffer.
pub fn execute_pipeline(source: &PixelBuffer<f32>, state: &EditState) -> PixelBuffer<f32> {
    // 1. Orientation & Flip
    let mut current = rotate_orthogonal(source, state.rotation);
    if state.flip_horizontal {
        flip_horizontal(&mut current);
    }
    if state.flip_vertical {
        flip_vertical(&mut current);
    }

    // 2. Crop
    if let Some(crop_rect) = state.crop {
        let (x, y, w, h) = crop_rect.to_pixel_coords(current.width, current.height);
        current = crop(&current, x, y, w, h);
    }

    // 3. Color & Tonal Adjustments
    apply_color_adjustments(&mut current, &state.color);

    // 4. Detail / Sharpening
    if state.sharpness > 0.0 {
        // Map 0.0..1.0 slider to radius 2, amount 0.0..1.5, threshold 0.01
        let amount = state.sharpness * 1.5;
        unsharp_mask(&mut current, 2, amount, 0.01);
    }

    current
}
