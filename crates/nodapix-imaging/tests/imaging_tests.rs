use nodapix_imaging::{
    apply_color_adjustments, crop, flip_horizontal, kelvin_to_white_balance_gains, linear_to_srgb,
    resize, rotate_orthogonal, rotate_straighten_autocrop, slider_to_kelvin, srgb_to_linear,
    straightened_size, unsharp_mask, ColorAdjustments, ExportFormat, ExportOptions, ImageBackend,
    OrthogonalRotation, PixelBuffer, RustImageBackend, TEMPERATURE_MAX_KELVIN,
    TEMPERATURE_MIN_KELVIN, TEMPERATURE_NEUTRAL_KELVIN,
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

#[test]
fn test_kelvin_slider_endpoints() {
    assert_eq!(slider_to_kelvin(0.0), TEMPERATURE_NEUTRAL_KELVIN);
    assert_eq!(slider_to_kelvin(1.0), TEMPERATURE_MAX_KELVIN);
    assert_eq!(slider_to_kelvin(-1.0), TEMPERATURE_MIN_KELVIN);
    // Out-of-range input is clamped rather than extrapolated.
    assert_eq!(slider_to_kelvin(4.0), TEMPERATURE_MAX_KELVIN);
}

#[test]
fn test_kelvin_gains_neutral_and_direction() {
    let neutral = kelvin_to_white_balance_gains(TEMPERATURE_NEUTRAL_KELVIN);
    for gain in neutral {
        assert!((gain - 1.0).abs() < 1e-6, "6500K must be unity gain, got {gain}");
    }

    let warm = kelvin_to_white_balance_gains(TEMPERATURE_MAX_KELVIN);
    assert!(warm[0] > warm[2], "high Kelvin must favour red: {warm:?}");

    let cool = kelvin_to_white_balance_gains(TEMPERATURE_MIN_KELVIN);
    assert!(cool[2] > cool[0], "low Kelvin must favour blue: {cool:?}");

    // The red/blue ratio moves monotonically with temperature.
    let mut last = f32::MIN;
    for kelvin in (2000..=12000).step_by(250) {
        let [r, _, b] = kelvin_to_white_balance_gains(kelvin as f32);
        let ratio = r / b;
        assert!(ratio > last, "red/blue ratio fell at {kelvin}K");
        last = ratio;
    }

    // Geometric-mean normalisation keeps the overall level near unity.
    let [r, g, b] = kelvin_to_white_balance_gains(TEMPERATURE_MIN_KELVIN);
    let mean = (r * g * b).cbrt();
    assert!((mean - 1.0).abs() < 1e-3, "gains must not change luminance: {mean}");
}

#[test]
fn test_temperature_neutral_by_default() {
    let adj = ColorAdjustments::default();
    assert!(adj.is_identity());
    assert_eq!(adj.temperature, TEMPERATURE_NEUTRAL_KELVIN);

    let mut buf = PixelBuffer::<f32>::new(1, 1);
    buf.set_pixel(0, 0, [0.5, 0.5, 0.5, 1.0]);
    apply_color_adjustments(&mut buf, &adj);
    assert_eq!(buf.get_pixel(0, 0), [0.5, 0.5, 0.5, 1.0]);
}

#[test]
fn test_hue_rotation_properties() {
    let luma = |p: [f32; 4]| 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];

    let mut buf = PixelBuffer::<f32>::new(1, 1);
    buf.set_pixel(0, 0, [1.0, 0.0, 0.0, 1.0]);

    // A full turn returns to the starting hue.
    let mut adj = ColorAdjustments::default();
    adj.hue = 1.0;
    apply_color_adjustments(&mut buf, &adj);
    let p = buf.get_pixel(0, 0);
    assert!((p[0] - 1.0).abs() < 1e-3, "360 degree turn should restore red: {p:?}");
    assert!(p[1].abs() < 1e-3 && p[2].abs() < 1e-3, "got {p:?}");

    // Third turns walk red through green and blue, preserving luminance.
    buf.set_pixel(0, 0, [1.0, 0.0, 0.0, 1.0]);
    adj.hue = 1.0 / 3.0;
    apply_color_adjustments(&mut buf, &adj);
    let toward_green = buf.get_pixel(0, 0);
    assert!(
        toward_green[1] > toward_green[0],
        "+120 degrees should push red into green: {toward_green:?}"
    );

    buf.set_pixel(0, 0, [1.0, 0.0, 0.0, 1.0]);
    adj.hue = -1.0 / 3.0;
    apply_color_adjustments(&mut buf, &adj);
    let toward_blue = buf.get_pixel(0, 0);
    assert!(
        toward_blue[2] > toward_blue[1],
        "-120 degrees should push red into blue: {toward_blue:?}"
    );

    // Luminance is preserved for a pixel that stays inside the RGB cube;
    // a saturated red rotated off its vertex would be clamped instead.
    buf.set_pixel(0, 0, [0.6, 0.5, 0.4, 1.0]);
    adj.hue = 1.0 / 3.0;
    apply_color_adjustments(&mut buf, &adj);
    let shifted = buf.get_pixel(0, 0);
    assert!(
        shifted[0] > 0.0 && shifted[0] < 1.0 && shifted[2] > 0.0,
        "test pixel must stay in gamut so nothing clamps: {shifted:?}"
    );
    assert!(
        (luma(shifted) - 0.5185).abs() < 1e-4,
        "hue rotation must not change luminance: {}",
        luma(shifted)
    );
}

#[test]
fn test_straighten_autocrops_without_transparent_corners() {
    let mut buf = PixelBuffer::<f32>::new(100, 50);
    buf.data.fill(0.25);
    for pixel in buf.data.chunks_exact_mut(4) {
        pixel[3] = 1.0;
    }

    assert_eq!(straightened_size(100, 50, 0.0), (100, 50));

    let rotated = rotate_straighten_autocrop(&buf, 30.0);
    assert_eq!(
        (rotated.width, rotated.height),
        straightened_size(100, 50, 30.0),
        "reported and actual canvas must agree"
    );
    assert!(rotated.width < 100 && rotated.height < 50);

    // Flat source: every pixel of the inscribed rectangle must be intact and opaque.
    for (x, y) in [
        (0, 0),
        (rotated.width - 1, 0),
        (0, rotated.height - 1),
        (rotated.width - 1, rotated.height - 1),
        (rotated.width / 2, rotated.height / 2),
    ] {
        let pixel = rotated.get_pixel(x, y);
        assert!(
            (pixel[0] - 0.25).abs() < 1e-3 && pixel[3] > 0.99,
            "inscribed canvas lost at ({x},{y}): {pixel:?}"
        );
    }
}

#[test]
fn test_straighten_direction() {
    // A horizon descending to the right at atan(0.2) ≈ 11.31°.
    let mut buf = PixelBuffer::<f32>::new(64, 64);
    for y in 0..64 {
        for x in 0..64 {
            let horizon = 32.0 + (x as f32 - 32.0) * 0.2;
            let v = if y as f32 > horizon { 0.9 } else { 0.1 };
            buf.set_pixel(x, y, [v, v, v, 1.0]);
        }
    }

    /// Rows the horizon drops across the frame: 0 once it is level.
    fn tilt(buffer: &PixelBuffer<f32>) -> i32 {
        let edge = |x: u32| {
            (0..buffer.height)
                .find(|&y| buffer.get_pixel(x, y)[0] > 0.5)
                .unwrap_or(buffer.height) as i32
        };
        (edge(2) - edge(buffer.width - 3)).abs()
    }

    assert!(tilt(&buf) > 10, "synthetic horizon should be tilted");
    // Negative degrees rotate counter-clockwise on screen, levelling it.
    assert!(
        tilt(&rotate_straighten_autocrop(&buf, -11.31)) <= 2,
        "-11.31° should level the horizon"
    );
    assert!(
        tilt(&rotate_straighten_autocrop(&buf, 11.31)) > tilt(&buf),
        "+11.31° should tilt it the other way, further"
    );
}

#[test]
fn test_straighten_zero_is_a_no_op() {
    let buf = PixelBuffer::<f32>::new(8, 6);
    assert_eq!(rotate_straighten_autocrop(&buf, 0.0), buf);
}

#[test]
fn test_resize_minifies_by_area_average() {
    let mut buf = PixelBuffer::<f32>::new(4, 4);
    for y in 0..4 {
        for x in 0..4 {
            let v = if (x + y) % 2 == 0 { 0.0 } else { 1.0 };
            buf.set_pixel(x, y, [v, v, v, 1.0]);
        }
    }

    let small = resize(&buf, 2, 2);
    assert_eq!((small.width, small.height), (2, 2));
    for y in 0..2 {
        for x in 0..2 {
            let p = small.get_pixel(x, y);
            assert!(
                (p[0] - 0.5).abs() < 1e-4,
                "checkerboard must average to mid-grey at ({x},{y}): {:?}",
                p
            );
        }
    }
}

#[test]
fn test_resize_upscale_interpolates() {
    let mut buf = PixelBuffer::<f32>::new(2, 1);
    buf.set_pixel(0, 0, [0.0, 0.0, 0.0, 1.0]);
    buf.set_pixel(1, 0, [1.0, 1.0, 1.0, 1.0]);

    let big = resize(&buf, 4, 1);
    let got: Vec<f32> = (0..4).map(|x| big.get_pixel(x, 0)[0]).collect();
    assert_eq!(got, [0.0, 1.0 / 4.0, 3.0 / 4.0, 1.0]);
}

#[test]
fn test_resize_same_size_returns_clone() {
    let mut buf = PixelBuffer::<f32>::new(3, 3);
    buf.set_pixel(1, 1, [0.7, 0.2, 0.1, 1.0]);
    assert_eq!(resize(&buf, 3, 3), buf);
}

#[test]
fn test_histogram_computation() {
    use nodapix_imaging::{compute_histogram, PixelBuffer};
    let mut buf = PixelBuffer::<u8>::new(10, 10);
    // Fill with 50% red [200, 0, 0, 255] and 50% white [255, 255, 255, 255]
    for y in 0..10 {
        for x in 0..10 {
            if x < 5 {
                buf.set_pixel(x, y, [200, 0, 0, 255]);
            } else {
                buf.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
    }

    let hist = compute_histogram(&buf);
    assert_eq!(hist.r[200], 50);
    assert_eq!(hist.r[255], 50);
    assert_eq!(hist.g[0], 50);
    assert_eq!(hist.g[255], 50);
    assert!(hist.max_count >= 50);
    // Highlight clipping should be triggered because 50% of pixels are at 255
    assert!(hist.highlight_clipped);
}

#[test]
fn test_auto_levels_calculation() {
    use nodapix_imaging::{calculate_auto_levels, PixelBuffer};
    let mut buf = PixelBuffer::<u8>::new(10, 10);
    // Flat low contrast grey image (all pixels between 100 and 140)
    for y in 0..10 {
        for x in 0..10 {
            let v = (100 + (x + y) * 2) as u8;
            buf.set_pixel(x, y, [v, v, v, 255]);
        }
    }

    let (exp_ev, contrast) = calculate_auto_levels(&buf);
    // Low contrast image should receive positive contrast boost
    assert!(contrast > 0.0);
    // Exposure shift should remain bounded
    assert!(exp_ev.abs() <= 1.0);
}

#[test]
fn test_crop_ratio_presets() {
    use nodapix_imaging::{CropRatioPreset, CropRect};

    // 16:9 preset on 1920x1080 (same ratio)
    let p_16_9 = CropRatioPreset::Ratio16x9;
    let r = p_16_9.target_ratio(1920.0, 1080.0, false).unwrap();
    assert!((r - (16.0 / 9.0)).abs() < 1e-4);
    let crop_same = CropRect::centered_with_ratio(r, 1920.0, 1080.0);
    assert_eq!(crop_same.width, 1.0);
    assert_eq!(crop_same.height, 1.0);

    // 1:1 Square on 1920x1080 image
    let p_sq = CropRatioPreset::Square;
    let r_sq = p_sq.target_ratio(1920.0, 1080.0, false).unwrap();
    assert_eq!(r_sq, 1.0);
    let crop_sq = CropRect::centered_with_ratio(r_sq, 1920.0, 1080.0);
    assert_eq!(crop_sq.height, 1.0);
    assert!((crop_sq.width - (1080.0 / 1920.0)).abs() < 1e-4);
    assert!((crop_sq.x - ((1.0 - crop_sq.width) / 2.0)).abs() < 1e-4);

    // Portrait 9:16 toggle
    let r_portrait = p_16_9.target_ratio(1920.0, 1080.0, true).unwrap();
    assert!((r_portrait - (9.0 / 16.0)).abs() < 1e-4);
}

#[test]
fn test_vignette_and_filter_presets() {
    use nodapix_imaging::{apply_color_adjustments, ColorAdjustments, FilterPreset, PixelBuffer};

    // Vignette test: center should remain brighter than corners
    let mut buf = PixelBuffer::<f32>::new(10, 10);
    buf.data.fill(0.8);
    let mut adj = ColorAdjustments::default();
    adj.vignette = 0.8;
    apply_color_adjustments(&mut buf, &adj);

    let center_px = buf.get_pixel(5, 5);
    let corner_px = buf.get_pixel(0, 0);
    assert!(
        center_px[0] > corner_px[0],
        "Center ({}) must be brighter than corner ({}) with vignette",
        center_px[0],
        corner_px[0]
    );

    // Black and white preset test: R, G, and B must be equal
    let mut col_buf = PixelBuffer::<f32>::new(2, 2);
    col_buf.set_pixel(0, 0, [0.9, 0.4, 0.2, 1.0]);
    let mut bw_adj = ColorAdjustments::default();
    bw_adj.filter_preset = FilterPreset::BlackAndWhite;
    apply_color_adjustments(&mut col_buf, &bw_adj);

    let px = col_buf.get_pixel(0, 0);
    assert!((px[0] - px[1]).abs() < 1e-4);
    assert!((px[1] - px[2]).abs() < 1e-4);
}

#[test]
fn test_image_metadata_inspection() {
    use nodapix_imaging::ImageMetadata;
    use std::path::PathBuf;

    // Test reading non-existent file returns None
    assert!(ImageMetadata::read_from_path(&PathBuf::from("/nonexistent/file.jpg")).is_none());

    // Test default metadata is empty
    let empty = ImageMetadata::default();
    assert!(empty.is_empty());
}
