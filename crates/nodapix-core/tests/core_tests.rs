use nodapix_core::ImageDocument;
use nodapix_imaging::PixelBuffer;

#[test]
fn test_document_history_undo_redo() {
    let mut buf = PixelBuffer::<f32>::new(10, 10);
    buf.data.fill(0.2);

    let mut doc = ImageDocument::new(buf, None);
    assert!(!doc.undo());
    assert!(!doc.redo());

    // State 1: rotate CW
    doc.commit_checkpoint();
    doc.state.rotate_cw();
    assert_eq!(doc.state.rotation, nodapix_imaging::OrthogonalRotation::Rotate90);

    // State 2: exposure +1.0
    doc.commit_checkpoint();
    doc.state.color.exposure = 1.0;

    // Undo exposure
    assert!(doc.undo());
    assert_eq!(doc.state.color.exposure, 0.0);
    assert_eq!(doc.state.rotation, nodapix_imaging::OrthogonalRotation::Rotate90);

    // Undo rotation
    assert!(doc.undo());
    assert_eq!(doc.state.rotation, nodapix_imaging::OrthogonalRotation::None);

    // Redo rotation
    assert!(doc.redo());
    assert_eq!(doc.state.rotation, nodapix_imaging::OrthogonalRotation::Rotate90);

    // Redo exposure
    assert!(doc.redo());
    assert_eq!(doc.state.color.exposure, 1.0);
}

#[test]
fn test_document_render_preview() {
    let mut buf = PixelBuffer::<f32>::new(4, 4);
    buf.set_pixel(0, 0, [1.0, 1.0, 1.0, 1.0]);

    let doc = ImageDocument::new(buf, None);
    let preview = doc.render_preview();
    assert_eq!(preview.width, 4);
    assert_eq!(preview.height, 4);

    let px = preview.get_pixel(0, 0);
    assert_eq!(px, [255, 255, 255, 255]);
}

#[test]
fn test_default_state_is_identity() {
    let doc = ImageDocument::new(PixelBuffer::<f32>::new(6, 4), None);
    assert!(doc.state.is_identity());
    assert_eq!(doc.output_size(), (6, 4));
}

#[test]
fn test_output_size_tracks_straightening() {
    let mut doc = ImageDocument::new(PixelBuffer::<f32>::new(200, 100), None);

    doc.state.angle_fine_degrees = 20.0;
    assert_eq!(
        doc.output_size(),
        nodapix_imaging::straightened_size(200, 100, 20.0)
    );

    // The document must report exactly what the pipeline renders.
    let rendered = doc.render_export();
    assert_eq!(
        (rendered.width, rendered.height),
        doc.output_size(),
        "output_size disagrees with the rendered canvas"
    );
}

#[test]
fn test_output_size_tracks_rotation_and_straightening() {
    let mut doc = ImageDocument::new(PixelBuffer::<f32>::new(200, 100), None);
    doc.state.rotate_cw();
    doc.state.angle_fine_degrees = 15.0;

    let expected = nodapix_imaging::straightened_size(100, 200, 15.0);
    assert_eq!(doc.output_size(), expected);

    let rendered = doc.render_export();
    assert_eq!((rendered.width, rendered.height), expected);
}

#[test]
fn test_straighten_keeps_aspect_and_leaves_no_gaps() {
    let mut buf = PixelBuffer::<f32>::new(64, 32);
    buf.data.fill(0.5);
    for pixel in buf.data.chunks_exact_mut(4) {
        pixel[3] = 1.0;
    }

    let mut doc = ImageDocument::new(buf, None);
    doc.state.angle_fine_degrees = 12.0;

    let out = doc.render_export();
    let ratio = out.width as f32 / out.height as f32;
    assert!(
        (ratio - 2.0).abs() < 0.05,
        "straightening must keep the 2:1 source aspect, got {ratio}"
    );
    assert!(out.width < 64 && out.height < 32, "inscribed crop must shrink");

    let centre = out.get_pixel(out.width / 2, out.height / 2);
    assert!(centre[0] > 0.4 && centre[0] < 0.6, "centre lost: {centre:?}");

    // The inscribed rectangle is fully covered: no transparent corners.
    for (x, y) in [(0, 0), (out.width - 1, 0), (0, out.height - 1), (out.width - 1, out.height - 1)] {
        let pixel = out.get_pixel(x, y);
        assert_eq!(pixel[3], 1.0, "straightened canvas should be opaque at ({x},{y})");
    }
}

#[test]
fn test_fine_angle_breaks_identity_and_resets() {
    let mut doc = ImageDocument::new(PixelBuffer::<f32>::new(8, 8), None);
    doc.state.angle_fine_degrees = 3.0;
    assert!(!doc.state.is_identity());

    doc.state.reset_all();
    assert!(doc.state.is_identity());
    assert_eq!(doc.state.angle_fine_degrees, 0.0);
}
