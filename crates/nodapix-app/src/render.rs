//! Bridges engine pixel buffers to GPUI GPU textures.

use gpui::RenderImage;
use image::Frame;
use nodapix_imaging::PixelBuffer;
use std::sync::Arc;

/// GPUI uploads `RenderImage` frames as BGRA with straight alpha, so the engine's
/// RGBA output is channel-swapped once at the texture boundary.
pub fn texture_from_rgba8(buffer: &PixelBuffer<u8>) -> Arc<RenderImage> {
    let mut data = buffer.data.clone();
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let frame = Frame::from_rgba8(buffer.width, buffer.height, data);
    Arc::new(RenderImage::new(vec![frame]))
}
