use nodapix_core::ImageDocument;
use nodapix_imaging::{
    CropRect, ExportFormat, ExportOptions, ImageBackend, OrthogonalRotation, RustImageBackend,
    straightened_size,
};
use std::path::PathBuf;

#[test]
fn test_real_image_full_pipeline_roundtrip() {
    let sample_path = PathBuf::from(
        "/Users/solution/Development/rea/pix/pix/extensions/webalbums/data/albumthemes/Classic/preview.png",
    );
    assert!(sample_path.exists(), "Sample test image must exist");

    let backend = RustImageBackend::new();
    let source_bytes = std::fs::read(&sample_path).expect("Read sample bytes");
    let original_buf = backend
        .decode_from_path(&sample_path)
        .expect("Decode real image");

    let orig_w = original_buf.width;
    let orig_h = original_buf.height;
    assert!(orig_w > 0 && orig_h > 0);

    let mut doc = ImageDocument::new(original_buf, Some(sample_path.clone()));

    // Verify proxy creation
    assert!(doc.proxy_source.width <= 2048);
    assert!(doc.proxy_source.height <= 2048);

    // Apply adjustments
    doc.commit_checkpoint();
    doc.state.rotation = OrthogonalRotation::Rotate90;
    doc.state.flip_horizontal = true;
    doc.state.crop = Some(CropRect {
        x: 0.1,
        y: 0.1,
        width: 0.8,
        height: 0.8,
    });
    doc.state.color.exposure = 0.5;
    doc.state.color.contrast = 0.2;
    doc.state.color.saturation = 0.3;
    doc.state.color.hue = 0.2;
    doc.state.color.temperature = 4200.0;
    doc.state.sharpness = 0.4;
    doc.state.angle_fine_degrees = 5.0;

    // Render preview
    let preview = doc.render_preview();
    assert!(preview.width > 0 && preview.height > 0);

    // Render full export
    let exported = doc.render_export();
    // In rotated 90 space, width is orig_h, height is orig_w; straightening
    // inscribes a crop before the user crop is applied.
    let (rot_w, rot_h) = straightened_size(orig_h, orig_w, doc.state.angle_fine_degrees);
    let expected_w = (rot_w as f32 * 0.8).round() as u32;
    let expected_h = (rot_h as f32 * 0.8).round() as u32;
    assert_eq!(exported.width, expected_w);
    assert_eq!(exported.height, expected_h);
    assert_eq!(
        doc.output_size(),
        (expected_w, expected_h),
        "output_size disagrees with the rendered export"
    );

    // Encode to PNG and JPEG
    let tmp_dir = std::env::temp_dir();
    let out_png = tmp_dir.join("nodapix_test_out.png");
    let out_jpg = tmp_dir.join("nodapix_test_out.jpg");

    backend
        .encode_to_path(
            &exported,
            &out_png,
            &ExportOptions {
                format: ExportFormat::Png,
                quality: 100,
            },
        )
        .expect("Encode PNG to disk");

    backend
        .encode_to_path(
            &exported,
            &out_jpg,
            &ExportOptions {
                format: ExportFormat::Jpeg,
                quality: 85,
            },
        )
        .expect("Encode JPEG to disk");

    assert!(out_png.exists());
    assert!(out_jpg.exists());

    // Verify decoded export
    let reloaded = backend.decode_from_path(&out_png).expect("Reload exported PNG");
    assert_eq!(reloaded.width, expected_w);
    assert_eq!(reloaded.height, expected_h);

    // Non-destructive editing is the core guarantee: exporting must never touch
    // the file the document was loaded from.
    assert_eq!(
        std::fs::read(&sample_path).expect("Re-read sample bytes"),
        source_bytes,
        "export modified the source file"
    );

    // Clean up
    let _ = std::fs::remove_file(out_png);
    let _ = std::fs::remove_file(out_jpg);
}
