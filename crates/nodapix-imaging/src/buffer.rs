//! PixelBuffer abstractions for NodaPix imaging engine.

use std::fmt::Debug;

/// Represents an image buffer stored contiguously in memory with 4 channels (RGBA).
#[derive(Clone, Debug, PartialEq)]
pub struct PixelBuffer<T> {
    pub width: u32,
    pub height: u32,
    pub data: Vec<T>,
}

impl<T: Copy + Default> PixelBuffer<T> {
    /// Create a new blank buffer initialized to default values.
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width as usize) * (height as usize) * 4;
        Self {
            width,
            height,
            data: vec![T::default(); size],
        }
    }

    /// Construct a buffer from an existing data vector.
    pub fn from_vec(width: u32, height: u32, data: Vec<T>) -> Result<Self, String> {
        let expected = (width as usize) * (height as usize) * 4;
        if data.len() != expected {
            return Err(format!(
                "Data length mismatch: expected {}, got {}",
                expected,
                data.len()
            ));
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    #[inline(always)]
    pub fn pixel_index(&self, x: u32, y: u32) -> usize {
        ((y as usize * self.width as usize) + x as usize) * 4
    }

    #[inline(always)]
    pub fn get_pixel(&self, x: u32, y: u32) -> [T; 4] {
        let idx = self.pixel_index(x, y);
        [
            self.data[idx],
            self.data[idx + 1],
            self.data[idx + 2],
            self.data[idx + 3],
        ]
    }

    #[inline(always)]
    pub fn set_pixel(&mut self, x: u32, y: u32, pixel: [T; 4]) {
        let idx = self.pixel_index(x, y);
        self.data[idx] = pixel[0];
        self.data[idx + 1] = pixel[1];
        self.data[idx + 2] = pixel[2];
        self.data[idx + 3] = pixel[3];
    }
}

impl PixelBuffer<f32> {
    /// Convert linear float RGBA [0.0, 1.0] to standard 8-bit sRGB RGBA.
    pub fn to_u8_srgb(&self) -> PixelBuffer<u8> {
        use rayon::prelude::*;

        let mut out = PixelBuffer::<u8>::new(self.width, self.height);
        out.data
            .par_chunks_exact_mut(4)
            .zip(self.data.par_chunks_exact(4))
            .for_each(|(dst, src)| {
                dst[0] = linear_to_srgb_u8(src[0]);
                dst[1] = linear_to_srgb_u8(src[1]);
                dst[2] = linear_to_srgb_u8(src[2]);
                dst[3] = (src[3].clamp(0.0, 1.0) * 255.0).round() as u8;
            });

        out
    }

    /// Convert 8-bit sRGB RGBA to linear float RGBA [0.0, 1.0].
    pub fn from_u8_srgb(source: &PixelBuffer<u8>) -> Self {
        use rayon::prelude::*;

        let mut out = PixelBuffer::<f32>::new(source.width, source.height);
        out.data
            .par_chunks_exact_mut(4)
            .zip(source.data.par_chunks_exact(4))
            .for_each(|(dst, src)| {
                dst[0] = srgb_u8_to_linear(src[0]);
                dst[1] = srgb_u8_to_linear(src[1]);
                dst[2] = srgb_u8_to_linear(src[2]);
                dst[3] = src[3] as f32 / 255.0;
            });

        out
    }

    /// Construct from an `image::DynamicImage`.
    pub fn from_dynamic_image(dyn_img: &image::DynamicImage) -> Self {
        let rgba = dyn_img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let u8_buf = PixelBuffer {
            width: w,
            height: h,
            data: rgba.into_raw(),
        };
        Self::from_u8_srgb(&u8_buf)
    }

    /// Export to an `image::RgbaImage` in sRGB.
    pub fn to_rgba_image(&self) -> image::RgbaImage {
        let u8_buf = self.to_u8_srgb();
        image::RgbaImage::from_raw(u8_buf.width, u8_buf.height, u8_buf.data)
            .expect("Valid dimensions")
    }
}

// Precise sRGB <-> Linear transfer functions
#[inline(always)]
pub fn srgb_to_linear(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

#[inline(always)]
pub fn linear_to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

#[inline(always)]
pub fn srgb_u8_to_linear(v: u8) -> f32 {
    srgb_to_linear(v as f32 / 255.0)
}

#[inline(always)]
pub fn linear_to_srgb_u8(v: f32) -> u8 {
    (linear_to_srgb(v) * 255.0).round().clamp(0.0, 255.0) as u8
}
