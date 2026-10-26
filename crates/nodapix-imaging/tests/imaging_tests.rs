use nodapix_imaging::{
    apply_color_adjustments, apply_exif_orientation, attach_exif, crop, flip_horizontal,
    kelvin_to_white_balance_gains, linear_to_srgb, resize, rotate_orthogonal,
    rotate_straighten_autocrop, slider_to_kelvin, srgb_to_linear, straightened_size, unsharp_mask,
    ColorAdjustments, ExportFormat, ExportOptions, ImageBackend, ImageMetadata, OrthogonalRotation,
    PixelBuffer, RustImageBackend, TEMPERATURE_MAX_KELVIN, TEMPERATURE_MIN_KELVIN,
    TEMPERATURE_NEUTRAL_KELVIN,
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

    // Negative / Invert test
    let mut inv_buf = PixelBuffer::<f32>::new(2, 2);
    inv_buf.set_pixel(0, 0, [0.8, 0.3, 0.1, 1.0]);
    let mut inv_adj = ColorAdjustments::default();
    inv_adj.invert = true;
    apply_color_adjustments(&mut inv_buf, &inv_adj);
    let ipx = inv_buf.get_pixel(0, 0);
    assert!((ipx[0] - 0.2).abs() < 1e-4);
    assert!((ipx[1] - 0.7).abs() < 1e-4);
    assert!((ipx[2] - 0.9).abs() < 1e-4);

    // Desaturate & Average B&W tests
    let mut desat_buf = PixelBuffer::<f32>::new(2, 2);
    desat_buf.set_pixel(0, 0, [0.9, 0.3, 0.1, 1.0]);
    let mut desat_adj = ColorAdjustments::default();
    desat_adj.filter_preset = FilterPreset::GrayscaleDesaturate;
    apply_color_adjustments(&mut desat_buf, &desat_adj);
    let dpx = desat_buf.get_pixel(0, 0);
    // (0.9 + 0.1) / 2 = 0.5
    assert!((dpx[0] - 0.5).abs() < 1e-4);
    assert_eq!(dpx[0], dpx[1]);
    assert_eq!(dpx[1], dpx[2]);

    // Lomo, Vintage, and CrossProcess should produce valid clamped pixels
    for preset in [FilterPreset::Lomo, FilterPreset::Vintage, FilterPreset::CrossProcess] {
        let mut f_buf = PixelBuffer::<f32>::new(2, 2);
        f_buf.set_pixel(0, 0, [0.6, 0.5, 0.4, 1.0]);
        let mut f_adj = ColorAdjustments::default();
        f_adj.filter_preset = preset;
        apply_color_adjustments(&mut f_buf, &f_adj);
        let fpx = f_buf.get_pixel(0, 0);
        assert!(fpx[0] >= 0.0 && fpx[0] <= 1.0);
        assert!(fpx[1] >= 0.0 && fpx[1] <= 1.0);
        assert!(fpx[2] >= 0.0 && fpx[2] <= 1.0);
    }
}

#[test]
fn test_white_balance_from_sample() {
    use nodapix_imaging::{
        calculate_white_balance_from_sample, TEMPERATURE_MAX_KELVIN, TEMPERATURE_MIN_KELVIN,
        TEMPERATURE_NEUTRAL_KELVIN,
    };

    // Neutral gray should return neutral temperature and tint
    let (k_neutral, t_neutral) = calculate_white_balance_from_sample(0.5, 0.5, 0.5);
    assert!((k_neutral - TEMPERATURE_NEUTRAL_KELVIN).abs() < 100.0);
    assert!(t_neutral.abs() < 0.05);

    // Warm tungsten cast (R=0.8, G=0.6, B=0.3): should decrease Kelvin to cool down
    let (k_warm, _) = calculate_white_balance_from_sample(0.8, 0.6, 0.3);
    assert!(k_warm < TEMPERATURE_NEUTRAL_KELVIN);
    assert!(k_warm >= TEMPERATURE_MIN_KELVIN);

    // Cool shadow/sky cast (R=0.3, G=0.5, B=0.8): should increase Kelvin to warm up
    let (k_cool, _) = calculate_white_balance_from_sample(0.3, 0.5, 0.8);
    assert!(k_cool > TEMPERATURE_NEUTRAL_KELVIN);
    assert!(k_cool <= TEMPERATURE_MAX_KELVIN);

    // Green cast (G=0.8, R=0.4, B=0.4): should produce positive tint (magenta correction)
    let (_, t_green) = calculate_white_balance_from_sample(0.4, 0.8, 0.4);
    assert!(t_green > 0.0);
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

/// A hand-built EXIF block: `Exif\0\0` + a little-endian TIFF with a Make string at
/// offset 38 and the orientation tag we want to exercise.
fn exif_block(orientation: u16) -> Vec<u8> {
    let mut tiff: Vec<u8> = Vec::new();
    tiff.extend_from_slice(b"II");
    tiff.extend_from_slice(&42u16.to_le_bytes());
    tiff.extend_from_slice(&8u32.to_le_bytes());
    tiff.extend_from_slice(&2u16.to_le_bytes());
    tiff.extend_from_slice(&0x010Fu16.to_le_bytes());
    tiff.extend_from_slice(&2u16.to_le_bytes());
    tiff.extend_from_slice(&6u32.to_le_bytes());
    tiff.extend_from_slice(&38u32.to_le_bytes());
    tiff.extend_from_slice(&0x0112u16.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&orientation.to_le_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_le_bytes());
    tiff.extend_from_slice(b"NIKON\0");
    assert_eq!(tiff.len(), 44);

    let mut block = b"Exif\0\0".to_vec();
    block.extend_from_slice(&tiff);
    block
}

fn gradient(width: u32, height: u32) -> PixelBuffer<f32> {
    let mut buf = PixelBuffer::<f32>::new(width, height);
    for y in 0..height {
        for x in 0..width {
            buf.set_pixel(x, y, [1.0, 0.0, 0.0, 1.0]);
        }
    }
    buf
}

#[test]
fn test_exif_block_survives_jpeg_export() {
    let backend = RustImageBackend::new();
    let opts = ExportOptions {
        format: ExportFormat::Jpeg,
        quality: 90,
    };
    let encoded = backend
        .encode_to_bytes(&gradient(20, 10), &opts)
        .expect("Encode JPEG");
    let block = exif_block(1);

    let spliced = attach_exif(&encoded, ExportFormat::Jpeg, &block);
    assert_eq!(
        ImageMetadata::read_exif_block(&spliced).as_deref(),
        Some(block.as_slice()),
        "the APP1 block must come back byte for byte"
    );

    let path = std::env::temp_dir().join(format!("nodapix-exif-{}.jpg", std::process::id()));
    std::fs::write(&path, &spliced).unwrap();
    let read = ImageMetadata::read_from_path(&path).expect("kamadak-exif must parse the spliced file");
    assert_eq!(read.make.as_deref(), Some("NIKON"));
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn test_exif_block_survives_png_export() {
    let backend = RustImageBackend::new();
    let opts = ExportOptions {
        format: ExportFormat::Png,
        quality: 100,
    };
    let encoded = backend
        .encode_to_bytes(&gradient(20, 10), &opts)
        .expect("Encode PNG");
    let block = exif_block(1);

    let spliced = attach_exif(&encoded, ExportFormat::Png, &block);
    assert_eq!(
        ImageMetadata::read_exif_block(&spliced).as_deref(),
        Some(block.as_slice())
    );
    // Decoding validates every chunk CRC, so this proves the eXIf chunk is well formed.
    let decoded = backend.decode_from_bytes(&spliced).expect("Decode PNG with eXIf");
    assert_eq!((decoded.width, decoded.height), (20, 10));
}

#[test]
fn test_orientation_tag_is_rewritten_in_place() {
    let mut block = exif_block(6);
    assert_eq!(ImageMetadata::orientation(&block), 6);
    ImageMetadata::set_orientation(&mut block, 1);
    assert_eq!(ImageMetadata::orientation(&block), 1);
    assert_eq!(block.len(), exif_block(6).len());
    assert_eq!(&block[6 + 38..6 + 44], b"NIKON\0");
}

#[test]
fn test_decode_applies_exif_orientation() {
    let backend = RustImageBackend::new();
    let opts = ExportOptions {
        format: ExportFormat::Png,
        quality: 100,
    };
    let encoded = backend
        .encode_to_bytes(&gradient(30, 10), &opts)
        .expect("Encode PNG");

    for (orientation, expected) in [(1_u32, (30, 10)), (6, (10, 30)), (8, (10, 30))] {
        let with_exif = attach_exif(
            &encoded,
            ExportFormat::Png,
            &exif_block(orientation as u16),
        );
        let decoded = backend
            .decode_from_bytes(&with_exif)
            .expect("decode oriented PNG");
        assert_eq!(
            (decoded.width, decoded.height),
            expected,
            "orientation {orientation} should swap the raster"
        );
    }
}

#[test]
fn test_apply_exif_orientation_matches_the_dihedral_map() {
    // 3x2 raster whose pixels encode their own coordinates, so any mapping is visible.
    let mut src = PixelBuffer::<u8>::new(3, 2);
    for y in 0..2u32 {
        for x in 0..3u32 {
            src.set_pixel(x, y, [x as u8 + 1, y as u8 + 1, 0, 255]);
        }
    }

    let rotated = apply_exif_orientation(&src, 6);
    assert_eq!((rotated.width, rotated.height), (2, 3));
    assert_eq!(rotated.get_pixel(0, 0)[0..2], [1, 2]);
    assert_eq!(rotated.get_pixel(1, 2)[0..2], [3, 1]);

    let transposed = apply_exif_orientation(&src, 5);
    assert_eq!((transposed.width, transposed.height), (2, 3));
    assert_eq!(transposed.get_pixel(0, 0)[0..2], [1, 1]);
    assert_eq!(transposed.get_pixel(1, 0)[0..2], [1, 2]);

    let flipped = apply_exif_orientation(&src, 2);
    assert_eq!((flipped.width, flipped.height), (3, 2));
    assert_eq!(flipped.get_pixel(0, 0)[0..2], [3, 1]);

    let unchanged = apply_exif_orientation(&src, 1);
    assert_eq!(unchanged.data, src.data);
}

#[test]
fn test_tone_curve_identity_and_evaluation() {
    use nodapix_imaging::{CurvePoint, ToneCurve};

    let curve = ToneCurve::default();
    assert!(curve.is_identity());
    assert_eq!(curve.evaluate(0.0), 0.0);
    assert_eq!(curve.evaluate(0.5), 0.5);
    assert_eq!(curve.evaluate(1.0), 1.0);

    // Custom S-curve
    let s_curve = ToneCurve::new(vec![
        CurvePoint::new(0.0, 0.0),
        CurvePoint::new(0.25, 0.15),
        CurvePoint::new(0.75, 0.85),
        CurvePoint::new(1.0, 1.0),
    ]);
    assert!(!s_curve.is_identity());
    assert!(s_curve.evaluate(0.25) < 0.25);
    assert!(s_curve.evaluate(0.75) > 0.75);

    // Check monotonicity across 100 sample points
    let mut prev = -1.0;
    for i in 0..=100 {
        let x = i as f32 / 100.0;
        let y = s_curve.evaluate(x);
        assert!(y >= prev, "Monotonicity violated at x={x}: y={y} < prev={prev}");
        assert!(y >= 0.0 && y <= 1.0, "Clamping violated at x={x}: y={y}");
        prev = y;
    }
}

#[test]
fn test_red_eye_removal_neutralization() {
    use nodapix_imaging::{apply_red_eye_fix_to_buffer, PixelBuffer, RedEyeFix};

    let mut buf = PixelBuffer::<f32>::new(50, 50);
    // Fill with neutral background
    buf.data.fill(0.3);

    // Create an intense flash pupil reflection at center (25, 25): bright red (R=0.9, G=0.1, B=0.1)
    buf.set_pixel(25, 25, [0.9, 0.1, 0.1, 1.0]);

    let fix = RedEyeFix {
        x: 0.5,
        y: 0.5,
        radius: 0.1, // 5 pixels on 50x50 image
    };

    apply_red_eye_fix_to_buffer(&mut buf, fix);

    let fixed_px = buf.get_pixel(25, 25);
    assert!(
        fixed_px[0] < 0.3,
        "Red channel should be neutralized down to iris tone: got {}",
        fixed_px[0]
    );
    // Green and blue channels should be preserved
    assert_eq!(fixed_px[1], 0.1);
    assert_eq!(fixed_px[2], 0.1);
}

#[test]
fn test_metadata_color_space() {
    let mut meta = ImageMetadata::default();
    assert!(meta.is_empty());
    assert_eq!(meta.color_space, None);

    meta.color_space = Some("sRGB".to_string());
    assert!(!meta.is_empty());
    assert_eq!(meta.color_space.as_deref(), Some("sRGB"));
}
