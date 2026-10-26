//! Image transformations: rotation, flip, crop, and resampling.

use crate::buffer::PixelBuffer;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OrthogonalRotation {
    #[default]
    None,
    Rotate90,
    Rotate180,
    Rotate270,
}

impl OrthogonalRotation {
    pub fn next_cw(self) -> Self {
        match self {
            Self::None => Self::Rotate90,
            Self::Rotate90 => Self::Rotate180,
            Self::Rotate180 => Self::Rotate270,
            Self::Rotate270 => Self::None,
        }
    }

    pub fn next_ccw(self) -> Self {
        match self {
            Self::None => Self::Rotate270,
            Self::Rotate90 => Self::None,
            Self::Rotate180 => Self::Rotate90,
            Self::Rotate270 => Self::Rotate180,
        }
    }
}

/// Normalized crop rectangle [0.0, 1.0] relative to current dimensions.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CropRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl CropRect {
    pub fn to_pixel_coords(&self, img_w: u32, img_h: u32) -> (u32, u32, u32, u32) {
        let x = (self.x * img_w as f32).round().clamp(0.0, img_w as f32 - 1.0) as u32;
        let y = (self.y * img_h as f32).round().clamp(0.0, img_h as f32 - 1.0) as u32;
        let w = (self.width * img_w as f32)
            .round()
            .clamp(1.0, (img_w - x) as f32) as u32;
        let h = (self.height * img_h as f32)
            .round()
            .clamp(1.0, (img_h - y) as f32) as u32;
        (x, y, w, h)
    }
}

/// Flip buffer horizontally in-place.
pub fn flip_horizontal<T: Copy + Send + Sync>(buffer: &mut PixelBuffer<T>) {
    let w = buffer.width as usize;
    buffer
        .data
        .par_chunks_exact_mut(w * 4)
        .for_each(|row| {
            for x in 0..(w / 2) {
                let left_idx = x * 4;
                let right_idx = (w - 1 - x) * 4;
                for c in 0..4 {
                    row.swap(left_idx + c, right_idx + c);
                }
            }
        });
}

/// Flip buffer vertically in-place.
pub fn flip_vertical<T: Copy + Send + Sync>(buffer: &mut PixelBuffer<T>) {
    let w = buffer.width as usize;
    let h = buffer.height as usize;
    let row_len = w * 4;

    for y in 0..(h / 2) {
        let top_start = y * row_len;
        let bot_start = (h - 1 - y) * row_len;
        let (first, second) = buffer.data.split_at_mut(bot_start);
        first[top_start..top_start + row_len].swap_with_slice(&mut second[..row_len]);
    }
}

/// Rotate buffer orthogonally (90, 180, 270 degrees clockwise).
pub fn rotate_orthogonal<T: Copy + Default + Send + Sync>(
    buffer: &PixelBuffer<T>,
    rotation: OrthogonalRotation,
) -> PixelBuffer<T> {
    match rotation {
        OrthogonalRotation::None => buffer.clone(),
        OrthogonalRotation::Rotate180 => {
            let mut out = buffer.clone();
            flip_horizontal(&mut out);
            flip_vertical(&mut out);
            out
        }
        OrthogonalRotation::Rotate90 => {
            let (dst_w, dst_h) = (buffer.height, buffer.width);
            let mut out = PixelBuffer::new(dst_w, dst_h);
            out.data
                .par_chunks_exact_mut(dst_w as usize * 4)
                .enumerate()
                .for_each(|(y_dst, row)| {
                    for x_dst in 0..dst_w as usize {
                        let src_x = y_dst;
                        let src_y = buffer.height as usize - 1 - x_dst;
                        let src_idx = (src_y * buffer.width as usize + src_x) * 4;
                        let dst_idx = x_dst * 4;
                        row[dst_idx..dst_idx + 4].copy_from_slice(&buffer.data[src_idx..src_idx + 4]);
                    }
                });
            out
        }
        OrthogonalRotation::Rotate270 => {
            let (dst_w, dst_h) = (buffer.height, buffer.width);
            let mut out = PixelBuffer::new(dst_w, dst_h);
            out.data
                .par_chunks_exact_mut(dst_w as usize * 4)
                .enumerate()
                .for_each(|(y_dst, row)| {
                    for x_dst in 0..dst_w as usize {
                        let src_x = buffer.width as usize - 1 - y_dst;
                        let src_y = x_dst;
                        let src_idx = (src_y * buffer.width as usize + src_x) * 4;
                        let dst_idx = x_dst * 4;
                        row[dst_idx..dst_idx + 4].copy_from_slice(&buffer.data[src_idx..src_idx + 4]);
                    }
                });
            out
        }
    }
}

/// Crop a rectangular region from the buffer.
pub fn crop<T: Copy + Default + Send + Sync>(
    buffer: &PixelBuffer<T>,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> PixelBuffer<T> {
    let mut out = PixelBuffer::new(width, height);
    let dst_stride = width as usize * 4;
    let copy_bytes = dst_stride;

    out.data
        .par_chunks_exact_mut(dst_stride)
        .enumerate()
        .for_each(|(dy, dst_row)| {
            let sy = (y as usize) + dy;
            let sx_start = ((sy * buffer.width as usize) + x as usize) * 4;
            dst_row.copy_from_slice(&buffer.data[sx_start..sx_start + copy_bytes]);
        });

    out
}

/// Downsample a buffer so that its longest edge is at most `max_dim`.
/// Returns the same buffer if it already fits.
pub fn downsample_to_fit(buffer: &PixelBuffer<f32>, max_dim: u32) -> PixelBuffer<f32> {
    let max_current = buffer.width.max(buffer.height);
    if max_current <= max_dim {
        return buffer.clone();
    }

    let scale = max_dim as f32 / max_current as f32;
    let dst_w = ((buffer.width as f32 * scale).round() as u32).max(1);
    let dst_h = ((buffer.height as f32 * scale).round() as u32).max(1);

    let mut out = PixelBuffer::new(dst_w, dst_h);
    let x_ratio = buffer.width as f32 / dst_w as f32;
    let y_ratio = buffer.height as f32 / dst_h as f32;

    out.data
        .par_chunks_exact_mut(dst_w as usize * 4)
        .enumerate()
        .for_each(|(dy, row)| {
            let sy = ((dy as f32 + 0.5) * y_ratio).clamp(0.0, buffer.height as f32 - 1.0) as u32;
            for dx in 0..dst_w as usize {
                let sx = ((dx as f32 + 0.5) * x_ratio).clamp(0.0, buffer.width as f32 - 1.0) as u32;
                let src_px = buffer.get_pixel(sx, sy);
                let dst_idx = dx * 4;
                row[dst_idx..dst_idx + 4].copy_from_slice(&src_px);
            }
        });

    out
}
