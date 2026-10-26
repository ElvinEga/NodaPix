use nodapix_imaging::{
    apply_color_adjustments, crop, flip_horizontal, linear_to_srgb, rotate_orthogonal,
    srgb_to_linear, unsharp_mask, ColorAdjustments, ExportFormat, ExportOptions, ImageBackend,
    OrthogonalRotation, PixelBuffer, RustImageBackend,
};

#[test]
fn test_srgb_linear_roundtrip() {
    for i in 0..=255 {
        let original = i as f32 / 255.0;
        let linear = srgb_to_linear(original);
        let back = linear_to_srgb(linear);
        assert!(
            (original - back).abs() < 1e-4,
            "Failed at {}: original={}, back={}",
            i,
            original,
            back
        );
    }
}

#[test]
fn test_color_adjustments_identity() {
    let mut buf = PixelBuffer::<f32>::new(10, 10);
    buf.data.fill(0.5);

    let default_adj = ColorAdjustments::default();
    apply_color_adjustments(&mut buf, &default_adj);

    for val in &buf.data {
        assert_eq!(*val, 0.5);
    }
}

#[test]
fn test_color_brightness_and_contrast() {
    let mut buf = PixelBuffer::<f32>::new(2, 2);
    buf.set_pixel(0, 0, [0.5, 0.5, 0.5, 1.0]);

    // Positive brightness shifts values towards 1.0
    let mut adj = ColorAdjustments::default();
    adj.brightness = 0.5;
    apply_color_adjustments(&mut buf, &adj);

    let px = buf.get_pixel(0, 0);
    assert!(px[0] > 0.5, "Brightness should lift value: got {}", px[0]);
    assert_eq!(px[0], 0.75); // lerp(0.5, 1.0, 0.5) = 0.75
}

#[test]
fn test_transform_flip_and_rotate() {
    // 2x3 buffer
    let mut buf = PixelBuffer::<f32>::new(2, 3);
    // (x=0, y=0) = [1.0, 0.0, 0.0, 1.0]
    buf.set_pixel(0, 0, [1.0, 0.0, 0.0, 1.0]);

    flip_horizontal(&mut buf);
    assert_eq!(buf.get_pixel(1, 0), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(buf.get_pixel(0, 0), [0.0, 0.0, 0.0, 0.0]);

    // Rotate 90 CW: (width=2, height=3) -> (width=3, height=2)
    let rot = rotate_orthogonal(&buf, OrthogonalRotation::Rotate90);
    assert_eq!(rot.width, 3);
    assert_eq!(rot.height, 2);
}

#[test]
fn test_crop_bounds() {
    let mut buf = PixelBuffer::<f32>::new(4, 4);
    buf.set_pixel(1, 1, [0.8, 0.8, 0.8, 1.0]);

    let cropped = crop(&buf, 1, 1, 2, 2);
    assert_eq!(cropped.width, 2);
    assert_eq!(cropped.height, 2);
    assert_eq!(cropped.get_pixel(0, 0), [0.8, 0.8, 0.8, 1.0]);
}

#[test]
fn test_unsharp_mask() {
    let mut buf = PixelBuffer::<f32>::new(16, 16);
    // Step edge in middle
    for y in 0..16 {
        for x in 8..16 {
            buf.set_pixel(x, y, [1.0, 1.0, 1.0, 1.0]);
        }
    }

    unsharp_mask(&mut buf, 2, 1.0, 0.0);
    // Edge contrast should be enhanced
    let left_edge = buf.get_pixel(7, 8);
    let right_edge = buf.get_pixel(8, 8);
    assert!(left_edge[0] <= 0.0);
    assert!(right_edge[0] >= 1.0);
}

#[test]
fn test_backend_png_encode_decode() {
    let backend = RustImageBackend::new();
    let mut buf = PixelBuffer::<f32>::new(8, 8);
    buf.set_pixel(0, 0, [1.0, 0.5, 0.25, 1.0]);

    let opts = ExportOptions {
        format: ExportFormat::Png,
        quality: 100,
    };
    let bytes = backend.encode_to_bytes(&buf, &opts).expect("Encode PNG");
    assert!(!bytes.is_empty());

    let decoded = backend.decode_from_bytes(&bytes).expect("Decode PNG");
    assert_eq!(decoded.width, 8);
    assert_eq!(decoded.height, 8);

    let px = decoded.get_pixel(0, 0);
    assert!((px[0] - 1.0).abs() < 0.05);
    assert!((px[1] - 0.5).abs() < 0.05);
}

#[cfg(target_os = "macos")]
#[test]
fn test_apple_image_io_decode() {
    use nodapix_imaging::AppleImageIOBackend;

    let sample_path = std::path::PathBuf::from(
        "/Users/solution/Development/rea/pix/pix/extensions/webalbums/data/albumthemes/Classic/preview.png",
    );
    let backend = AppleImageIOBackend::new();
    let decoded = backend.decode_from_path(&sample_path).expect("Apple Image I/O decode");
    assert!(decoded.width > 0 && decoded.height > 0);
}
