use nodapix_core::ImageDocument;
use nodapix_imaging::{ExportFormat, ExportOptions, ImageBackend, RustImageBackend};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        println!("NodaPix Image Editor for macOS (Apple Silicon native)");
        println!("Usage: nodapix <image_path> [--rotate 90|180|270] [--exposure <ev>] [--contrast <factor>] [--out <output_path>]");
        println!("\nNo image supplied. Running in interactive mode...");
        return;
    }

    let input_path = PathBuf::from(&args[1]);
    if !input_path.exists() {
        eprintln!("Error: Image file does not exist: {:?}", input_path);
        std::process::exit(1);
    }

    let backend = RustImageBackend::new();
    let original = match backend.decode_from_path(&input_path) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("Failed to decode image: {}", e);
            std::process::exit(1);
        }
    };

    println!(
        "Loaded image: {:?} ({}x{} px)",
        input_path, original.width, original.height
    );

    let mut doc = ImageDocument::new(original, Some(input_path));

    // Simple CLI flag parser for batch or quick testing
    let mut out_path = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--rotate" => {
                if let Some(deg) = args.get(i + 1) {
                    match deg.as_str() {
                        "90" => doc.state.rotation = nodapix_imaging::OrthogonalRotation::Rotate90,
                        "180" => doc.state.rotation = nodapix_imaging::OrthogonalRotation::Rotate180,
                        "270" => doc.state.rotation = nodapix_imaging::OrthogonalRotation::Rotate270,
                        _ => eprintln!("Unsupported rotation: {}", deg),
                    }
                    i += 1;
                }
            }
            "--exposure" => {
                if let Some(val) = args.get(i + 1) {
                    doc.state.color.exposure = val.parse().unwrap_or(0.0);
                    i += 1;
                }
            }
            "--contrast" => {
                if let Some(val) = args.get(i + 1) {
                    doc.state.color.contrast = val.parse().unwrap_or(0.0);
                    i += 1;
                }
            }
            "--out" => {
                if let Some(val) = args.get(i + 1) {
                    out_path = Some(PathBuf::from(val));
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    if let Some(out) = out_path {
        let ext = out.extension().and_then(|s| s.to_str()).unwrap_or("png");
        let format = ExportFormat::from_extension(ext).unwrap_or(ExportFormat::Png);
        let rendered = doc.render_export();
        backend
            .encode_to_path(&rendered, &out, &ExportOptions { format, quality: 90 })
            .expect("Export image to destination");
        println!("Exported edited image to: {:?} ({}x{})", out, rendered.width, rendered.height);
    } else {
        let preview = doc.render_preview();
        println!("Computed preview buffer: {}x{} px", preview.width, preview.height);
    }
}
