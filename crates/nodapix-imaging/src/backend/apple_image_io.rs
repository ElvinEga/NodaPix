//! Apple Image I/O backend for native hardware-accelerated image decoding on macOS.
//! Supports Apple HEIC/HEIF, camera RAW (Apple ProRAW, DNG, CR2, NEF, ARW), and standard formats.

use crate::backend::{ExportOptions, ImageBackend, ImageError, RustImageBackend};
use crate::buffer::PixelBuffer;
use std::ffi::c_void;
use std::path::Path;

#[repr(C)]
struct __CFData(c_void);
type CFDataRef = *const __CFData;

#[repr(C)]
struct __CGImageSource(c_void);
type CGImageSourceRef = *const __CGImageSource;

#[repr(C)]
struct __CGImage(c_void);
type CGImageRef = *const __CGImage;

#[repr(C)]
struct __CGColorSpace(c_void);
type CGColorSpaceRef = *const __CGColorSpace;

#[repr(C)]
struct __CGContext(c_void);
type CGContextRef = *const __CGContext;

type CFIndex = isize;
type CGFloat = f64;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: CGFloat,
    y: CGFloat,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: CGFloat,
    height: CGFloat,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

// macOS constants for 32-bit RGBA
const K_CG_IMAGE_ALPHA_PREMULTIPLIED_LAST: u32 = 1;
const K_CG_BITMAP_BYTE_ORDER_32_BIG: u32 = 4 << 12;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFDataCreate(allocator: *const c_void, bytes: *const u8, length: CFIndex) -> CFDataRef;
    fn CFRelease(cf: *const c_void);
}

#[link(name = "ImageIO", kind = "framework")]
extern "C" {
    fn CGImageSourceCreateWithData(data: CFDataRef, options: *const c_void) -> CGImageSourceRef;
    fn CGImageSourceCreateImageAtIndex(
        isrc: CGImageSourceRef,
        index: usize,
        options: *const c_void,
    ) -> CGImageRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGImageGetWidth(image: CGImageRef) -> usize;
    fn CGImageGetHeight(image: CGImageRef) -> usize;
    fn CGColorSpaceCreateDeviceRGB() -> CGColorSpaceRef;
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: CGColorSpaceRef,
        bitmap_info: u32,
    ) -> CGContextRef;
    fn CGContextDrawImage(c: CGContextRef, rect: CGRect, image: CGImageRef);
    fn CGContextRelease(c: CGContextRef);
    fn CGColorSpaceRelease(space: CGColorSpaceRef);
    fn CGImageRelease(image: CGImageRef);
}

/// Native macOS Image I/O decoder and fallback Rust encoder.
#[derive(Default, Clone, Debug)]
pub struct AppleImageIOBackend {
    fallback: RustImageBackend,
}

impl AppleImageIOBackend {
    pub fn new() -> Self {
        Self {
            fallback: RustImageBackend::new(),
        }
    }
}

impl ImageBackend for AppleImageIOBackend {
    fn decode_from_bytes(&self, bytes: &[u8]) -> Result<PixelBuffer<f32>, ImageError> {
        if bytes.is_empty() {
            return Err(ImageError::Unsupported("Empty buffer".to_string()));
        }

        unsafe {
            let cf_data = CFDataCreate(std::ptr::null(), bytes.as_ptr(), bytes.len() as CFIndex);
            if cf_data.is_null() {
                return self.fallback.decode_from_bytes(bytes);
            }

            let source = CGImageSourceCreateWithData(cf_data, std::ptr::null());
            if source.is_null() {
                CFRelease(cf_data as *const c_void);
                return self.fallback.decode_from_bytes(bytes);
            }

            let cg_image = CGImageSourceCreateImageAtIndex(source, 0, std::ptr::null());
            if cg_image.is_null() {
                CFRelease(source as *const c_void);
                CFRelease(cf_data as *const c_void);
                return self.fallback.decode_from_bytes(bytes);
            }

            let width = CGImageGetWidth(cg_image);
            let height = CGImageGetHeight(cg_image);

            if width == 0 || height == 0 {
                CGImageRelease(cg_image);
                CFRelease(source as *const c_void);
                CFRelease(cf_data as *const c_void);
                return Err(ImageError::Unsupported("Zero-sized image".to_string()));
            }

            let mut raw_bytes = vec![0_u8; width * height * 4];
            let color_space = CGColorSpaceCreateDeviceRGB();
            let bitmap_info = K_CG_IMAGE_ALPHA_PREMULTIPLIED_LAST | K_CG_BITMAP_BYTE_ORDER_32_BIG;

            let context = CGBitmapContextCreate(
                raw_bytes.as_mut_ptr() as *mut c_void,
                width,
                height,
                8,
                width * 4,
                color_space,
                bitmap_info,
            );

            if context.is_null() {
                CGColorSpaceRelease(color_space);
                CGImageRelease(cg_image);
                CFRelease(source as *const c_void);
                CFRelease(cf_data as *const c_void);
                return self.fallback.decode_from_bytes(bytes);
            }

            let rect = CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: CGSize {
                    width: width as CGFloat,
                    height: height as CGFloat,
                },
            };

            CGContextDrawImage(context, rect, cg_image);

            // Cleanup CoreGraphics references
            CGContextRelease(context);
            CGColorSpaceRelease(color_space);
            CGImageRelease(cg_image);
            CFRelease(source as *const c_void);
            CFRelease(cf_data as *const c_void);

            let u8_buffer = PixelBuffer {
                width: width as u32,
                height: height as u32,
                data: raw_bytes,
            };

            Ok(PixelBuffer::from_u8_srgb(&u8_buffer))
        }
    }

    fn decode_from_path(&self, path: &Path) -> Result<PixelBuffer<f32>, ImageError> {
        let bytes = std::fs::read(path)?;
        self.decode_from_bytes(&bytes)
    }

    fn encode_to_bytes(
        &self,
        buffer: &PixelBuffer<f32>,
        options: &ExportOptions,
    ) -> Result<Vec<u8>, ImageError> {
        self.fallback.encode_to_bytes(buffer, options)
    }

    fn encode_to_path(
        &self,
        buffer: &PixelBuffer<f32>,
        path: &Path,
        options: &ExportOptions,
    ) -> Result<(), ImageError> {
        self.fallback.encode_to_path(buffer, path, options)
    }
}
