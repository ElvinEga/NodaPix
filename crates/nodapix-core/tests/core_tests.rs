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
