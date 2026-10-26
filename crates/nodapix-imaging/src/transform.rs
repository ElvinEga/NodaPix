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

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CropRatioPreset {
    #[default]
    Free,
    Square,     // 1:1
    Original,   // matches current image aspect ratio
    Ratio3x2,   // 3:2
    Ratio4x3,   // 4:3
    Ratio16x9,  // 16:9
    Ratio16x10, // 16:10
}

impl CropRatioPreset {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Free => "Free",
            Self::Square => "1:1",
            Self::Original => "Original",
            Self::Ratio3x2 => "3:2",
            Self::Ratio4x3 => "4:3",
            Self::Ratio16x9 => "16:9",
            Self::Ratio16x10 => "16:10",
        }
    }

    pub fn target_ratio(&self, img_w: f32, img_h: f32, portrait: bool) -> Option<f32> {
        let base = match self {
            Self::Free => return None,
            Self::Square => 1.0,
            Self::Original => {
                if img_h > 0.0 {
                    img_w / img_h
                } else {
                    1.0
                }
            }
            Self::Ratio3x2 => 3.0 / 2.0,
            Self::Ratio4x3 => 4.0 / 3.0,
            Self::Ratio16x9 => 16.0 / 9.0,
            Self::Ratio16x10 => 16.0 / 10.0,
        };

        if portrait {
            Some(1.0 / base)
        } else {
            Some(base)
        }
    }
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

    /// Construct a centered crop rectangle with a given aspect ratio within image bounds.
    pub fn centered_with_ratio(ratio: f32, img_w: f32, img_h: f32) -> Self {
        if img_w <= 0.0 || img_h <= 0.0 || ratio <= 0.0 {
            return Self {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            };
        }
        let img_ratio = img_w / img_h;
        let (cw, ch) = if ratio >= img_ratio {
            let h = (img_ratio / ratio).clamp(0.05, 1.0);
            (1.0, h)
        } else {
            let w = (ratio / img_ratio).clamp(0.05, 1.0);
            (w, 1.0)
        };
        let cx = ((1.0 - cw) / 2.0).clamp(0.0, 1.0 - cw);
        let cy = ((1.0 - ch) / 2.0).clamp(0.0, 1.0 - ch);
        Self {
            x: cx,
            y: cy,
            width: cw,
            height: ch,
        }
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
    resize(buffer, dst_w, dst_h)
}

/// Destination taps for one axis: for each output index, the source indices to
/// blend and their weights.
///
/// The filter radius grows with the downscale factor so minifying averages the
/// source area it covers instead of discarding pixels; at or below 1:1 the
/// radius collapses to exactly bilinear weights.
fn axis_taps(src: usize, dst: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = src as f32 / dst as f32;
    let radius = scale.max(1.0) * 0.5;
    (0..dst)
        .map(|d| {
            let centre = (d as f32 + 0.5) * scale - 0.5;
            // A source pixel contributes when its [s-0.5, s+0.5] span overlaps
            // the filter window, so the inclusive range is open at both ends.
            let lo = (centre - radius - 0.5).floor() as i64 + 1;
            let hi = (centre + radius + 0.5).ceil() as i64 - 1;
            let mut taps = Vec::new();
            let mut total = 0.0f32;
            for s in lo..=hi {
                if s < 0 || s as usize >= src {
                    continue;
                }
                // overlap of [s-0.5, s+0.5] with the filter window [centre-radius, centre+radius]
                let overlap =
                    (s as f32 + 0.5).min(centre + radius) - (s as f32 - 0.5).max(centre - radius);
                let weight = overlap.max(0.0);
                if weight > 0.0 {
                    total += weight;
                    taps.push((s as usize, weight));
                }
            }
            if taps.is_empty() {
                let nearest = centre.round().clamp(0.0, src as f32 - 1.0) as usize;
                taps.push((nearest, 1.0));
            } else {
                for (_, w) in taps.iter_mut() {
                    *w /= total;
                }
            }
            taps
        })
        .collect()
}

/// Resize with a separable box/bilinear filter, channel by channel.
pub fn resize(buffer: &PixelBuffer<f32>, dst_w: u32, dst_h: u32) -> PixelBuffer<f32> {
    let dst_w = dst_w.max(1) as usize;
    let dst_h = dst_h.max(1) as usize;
    if buffer.width as usize == dst_w && buffer.height as usize == dst_h {
        return buffer.clone();
    }

    let src_w = buffer.width as usize;
    let src_h = buffer.height as usize;
    let x_taps = axis_taps(src_w, dst_w);
    let y_taps = axis_taps(src_h, dst_h);

    // Horizontal pass: src_h rows swept into a dst_w x src_h buffer.
    let mut swept = vec![0.0f32; dst_w * src_h * 4];
    for y in 0..src_h {
        let src_row = y * src_w * 4;
        let dst_row = y * dst_w * 4;
        for (dx, taps) in x_taps.iter().enumerate() {
            let out = &mut swept[dst_row + dx * 4..dst_row + dx * 4 + 4];
            for (sx, weight) in taps {
                let base = src_row + sx * 4;
                for c in 0..4 {
                    out[c] += buffer.data[base + c] * weight;
                }
            }
        }
    }

    let mut out = PixelBuffer::new(dst_w as u32, dst_h as u32);
    for (dy, taps) in y_taps.iter().enumerate() {
        let dst_base = dy * dst_w * 4;
        for (sy, weight) in taps {
            let src_base = sy * dst_w * 4;
            for i in 0..dst_w * 4 {
                out.data[dst_base + i] += swept[src_base + i] * weight;
            }
        }
    }

    out
}

/// Output canvas for straightening by `degrees`: the largest rectangle of the
/// source aspect ratio that fits inside the rotated frame, so no transparent
/// corners are ever produced.
///
/// [`rotate_straighten_autocrop`] derives its own buffer from this, and the
/// document reports it as the output size; they must not diverge.
pub fn straightened_size(width: u32, height: u32, degrees: f32) -> (u32, u32) {
    if degrees.abs() < 1e-3 {
        return (width, height);
    }
    let src_w = width as f32;
    let src_h = height as f32;
    let aspect = src_w / src_h;
    let abs_rad = degrees.abs().to_radians();
    let (sin, cos) = abs_rad.sin_cos();
    let dst_w = (src_w / (cos + aspect * sin)).floor().max(1.0);
    let dst_h = (dst_w / aspect).floor().max(1.0);
    (dst_w as u32, dst_h as u32)
}

/// Rotate with automatic inscribed crop, eliminating transparent corner gaps.
/// Keeps the original aspect ratio while straightening horizon by `degrees` (-45° to +45°).
pub fn rotate_straighten_autocrop(buffer: &PixelBuffer<f32>, degrees: f32) -> PixelBuffer<f32> {
    if degrees.abs() < 1e-3 {
        return buffer.clone();
    }

    let src_w = buffer.width as f32;
    let src_h = buffer.height as f32;
    let (dst_w, dst_h) = straightened_size(buffer.width, buffer.height, degrees);
    let dst_w_f = dst_w as f32;
    let dst_h_f = dst_h as f32;

    let (sin, cos) = degrees.to_radians().sin_cos();

    let mut out = PixelBuffer::new(dst_w, dst_h);
    out.data
        .par_chunks_exact_mut(dst_w as usize * 4)
        .enumerate()
        .for_each(|(dy, row)| {
            let qy = dy as f32 + 0.5 - dst_h_f * 0.5;
            for dx in 0..dst_w as usize {
                let qx = dx as f32 + 0.5 - dst_w_f * 0.5;
                let sx = (qx * cos + qy * sin + src_w * 0.5).clamp(0.0, src_w - 1.0);
                let sy = (-qx * sin + qy * cos + src_h * 0.5).clamp(0.0, src_h - 1.0);

                let x0 = sx.floor() as u32;
                let y0 = sy.floor() as u32;
                let x1 = (x0 + 1).min(buffer.width - 1);
                let y1 = (y0 + 1).min(buffer.height - 1);
                let fx = sx - x0 as f32;
                let fy = sy - y0 as f32;

                let p00 = buffer.get_pixel(x0, y0);
                let p10 = buffer.get_pixel(x1, y0);
                let p01 = buffer.get_pixel(x0, y1);
                let p11 = buffer.get_pixel(x1, y1);

                let out_px = &mut row[dx * 4..dx * 4 + 4];
                for c in 0..4 {
                    let top = p00[c] + (p10[c] - p00[c]) * fx;
                    let bottom = p01[c] + (p11[c] - p01[c]) * fx;
                    out_px[c] = top + (bottom - top) * fy;
                }
            }
        });

    out
}
