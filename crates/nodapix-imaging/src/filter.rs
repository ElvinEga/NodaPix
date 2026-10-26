//! Spatial filters: Gaussian blur approximation and Unsharp Masking.

use crate::buffer::PixelBuffer;
use rayon::prelude::*;

/// 1D horizontal box blur pass on f32 RGBA buffer using moving accumulator.
fn box_blur_h(src: &PixelBuffer<f32>, dst: &mut PixelBuffer<f32>, radius: usize) {
    let w = src.width as usize;
    let _h = src.height as usize;
    let inv_window = 1.0 / ((2 * radius + 1) as f32);

    dst.data
        .par_chunks_exact_mut(w * 4)
        .enumerate()
        .for_each(|(y, dst_row)| {
            let row_offset = y * w * 4;
            let mut acc = [0.0_f32; 4];

            // Initial accumulator from boundary clamp
            let first_px = [
                src.data[row_offset],
                src.data[row_offset + 1],
                src.data[row_offset + 2],
                src.data[row_offset + 3],
            ];
            for c in 0..4 {
                acc[c] = first_px[c] * (radius + 1) as f32;
            }
            for x in 1..=radius {
                let px_x = x.min(w - 1);
                let idx = row_offset + px_x * 4;
                for c in 0..4 {
                    acc[c] += src.data[idx + c];
                }
            }

            for x in 0..w {
                let dst_idx = x * 4;
                for c in 0..4 {
                    dst_row[dst_idx + c] = acc[c] * inv_window;
                }

                // Move sliding window: add incoming, subtract outgoing
                let in_x = (x + radius + 1).min(w - 1);
                let out_x = (x as isize - radius as isize).max(0) as usize;
                let in_idx = row_offset + in_x * 4;
                let out_idx = row_offset + out_x * 4;

                for c in 0..4 {
                    acc[c] += src.data[in_idx + c] - src.data[out_idx + c];
                }
            }
        });
}

/// 1D vertical box blur pass on f32 RGBA buffer.
fn box_blur_v(src: &PixelBuffer<f32>, dst: &mut PixelBuffer<f32>, radius: usize) {
    let w = src.width as usize;
    let h = src.height as usize;
    let inv_window = 1.0 / ((2 * radius + 1) as f32);

    // Process columns in parallel chunks
    (0..w).into_par_iter().for_each(|x| {
        let mut acc = [0.0_f32; 4];
        let first_idx = x * 4;
        let first_px = [
            src.data[first_idx],
            src.data[first_idx + 1],
            src.data[first_idx + 2],
            src.data[first_idx + 3],
        ];

        for c in 0..4 {
            acc[c] = first_px[c] * (radius + 1) as f32;
        }
        for y in 1..=radius {
            let py = y.min(h - 1);
            let idx = (py * w + x) * 4;
            for c in 0..4 {
                acc[c] += src.data[idx + c];
            }
        }

        for y in 0..h {
            let dst_idx = (y * w + x) * 4;
            // Unsafe direct write or slice write for column elements
            // To maintain safety without per-column lock, we write through raw pointer or chunk
            // Since x is distinct across threads, each thread writes exclusively to disjoint indices
            unsafe {
                let ptr = dst.data.as_ptr() as *mut f32;
                for c in 0..4 {
                    *ptr.add(dst_idx + c) = acc[c] * inv_window;
                }
            }

            let in_y = (y + radius + 1).min(h - 1);
            let out_y = (y as isize - radius as isize).max(0) as usize;
            let in_idx = (in_y * w + x) * 4;
            let out_idx = (out_y * w + x) * 4;

            for c in 0..4 {
                acc[c] += src.data[in_idx + c] - src.data[out_idx + c];
            }
        }
    });
}

/// 3-pass box blur approximating Gaussian Blur with radius `r`.
pub fn gaussian_blur(src: &PixelBuffer<f32>, radius: usize) -> PixelBuffer<f32> {
    if radius == 0 {
        return src.clone();
    }

    let mut temp = PixelBuffer::new(src.width, src.height);
    let mut current = src.clone();

    // 3 iterations of horizontal + vertical box blur
    for _ in 0..3 {
        box_blur_h(&current, &mut temp, radius);
        box_blur_v(&temp, &mut current, radius);
    }

    current
}

/// Unsharp Masking filter based on Pix algorithm.
/// - `amount`: Sharpening strength (0.0 to 2.0)
/// - `radius`: Blur radius in pixels (1 to 15)
/// - `threshold`: Minimum difference (0.0 to 0.2) to sharpen, suppressing noise amplification
pub fn unsharp_mask(
    buffer: &mut PixelBuffer<f32>,
    radius: usize,
    amount: f32,
    threshold: f32,
) {
    if amount <= 0.0 || radius == 0 {
        return;
    }

    let blurred = gaussian_blur(buffer, radius);

    buffer
        .data
        .par_chunks_exact_mut(4)
        .zip(blurred.data.par_chunks_exact(4))
        .for_each(|(src, blur)| {
            for c in 0..3 {
                let diff = src[c] - blur[c];
                if diff.abs() >= threshold {
                    src[c] = (src[c] + amount * diff).clamp(0.0, 1.0);
                }
            }
        });
}
