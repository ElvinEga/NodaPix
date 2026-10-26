//! Multi-image contact sheet generation.
//! Ports Pix's contact sheet extension with grid layout, headers, captions, and export.

use crate::backend::{ExportFormat, ExportOptions, ImageBackend, NativeBackend};
use crate::buffer::PixelBuffer;
use crate::font::{render_text, text_dimensions, truncate_to_width};
use crate::transform::resize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SheetBackground {
    White,
    DarkSlate,
    Black,
}

impl SheetBackground {
    pub fn color(self) -> [f32; 4] {
        match self {
            Self::White => [1.0, 1.0, 1.0, 1.0],
            Self::DarkSlate => [0.12, 0.13, 0.18, 1.0],
            Self::Black => [0.0, 0.0, 0.0, 1.0],
        }
    }

    pub fn text_color(self) -> [f32; 4] {
        match self {
            Self::White => [0.1, 0.1, 0.1, 1.0],
            Self::DarkSlate => [0.92, 0.92, 0.94, 1.0],
            Self::Black => [0.95, 0.95, 0.95, 1.0],
        }
    }

    pub fn border_color(self) -> [f32; 4] {
        match self {
            Self::White => [0.8, 0.8, 0.85, 1.0],
            Self::DarkSlate => [0.25, 0.27, 0.35, 1.0],
            Self::Black => [0.25, 0.25, 0.25, 1.0],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::White => "White",
            Self::DarkSlate => "Dark Slate",
            Self::Black => "Black",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetPreset {
    Standard4x3, // 2400 x 1800
    Photo3x2,    // 3000 x 2000
    Square1x1,   // 2400 x 2400
    Wide16x9,    // 1920 x 1080
}

impl SheetPreset {
    pub fn dimensions(self) -> (usize, usize) {
        match self {
            Self::Standard4x3 => (2400, 1800),
            Self::Photo3x2 => (3000, 2000),
            Self::Square1x1 => (2400, 2400),
            Self::Wide16x9 => (1920, 1080),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Standard4x3 => "Standard 4:3 (2400×1800)",
            Self::Photo3x2 => "Photo 3:2 (3000×2000)",
            Self::Square1x1 => "Square 1:1 (2400×2400)",
            Self::Wide16x9 => "Widescreen 16:9 (1920×1080)",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ContactSheetConfig {
    pub cols: usize,
    pub rows: usize,
    pub width: usize,
    pub height: usize,
    pub background: SheetBackground,
    pub show_filenames: bool,
    pub title: Option<String>,
    pub margin: usize,
    pub spacing: usize,
    pub format: ExportFormat,
    pub quality: u8,
}

impl Default for ContactSheetConfig {
    fn default() -> Self {
        let (w, h) = SheetPreset::Standard4x3.dimensions();
        Self {
            cols: 4,
            rows: 4,
            width: w,
            height: h,
            background: SheetBackground::White,
            show_filenames: true,
            title: Some("NodaPix Contact Sheet".to_string()),
            margin: 48,
            spacing: 24,
            format: ExportFormat::Png,
            quality: 90,
        }
    }
}

/// Composite a source buffer into the destination buffer at `(dst_x, dst_y)`.
pub fn blit_buffer(
    dest: &mut PixelBuffer<f32>,
    src: &PixelBuffer<f32>,
    dst_x: usize,
    dst_y: usize,
) {
    let copy_w = src.width as usize;
    let copy_h = src.height as usize;
    for y in 0..copy_h {
        let dy = dst_y + y;
        if dy >= dest.height as usize {
            break;
        }
        for x in 0..copy_w {
            let dx = dst_x + x;
            if dx >= dest.width as usize {
                break;
            }
            let src_px = src.get_pixel(x as u32, y as u32);
            let dst_px = dest.get_pixel(dx as u32, dy as u32);
            let alpha = src_px[3];
            let inv_alpha = 1.0 - alpha;
            let blended = [
                src_px[0] * alpha + dst_px[0] * inv_alpha,
                src_px[1] * alpha + dst_px[1] * inv_alpha,
                src_px[2] * alpha + dst_px[2] * inv_alpha,
                alpha + dst_px[3] * inv_alpha,
            ];
            dest.set_pixel(dx as u32, dy as u32, blended);
        }
    }
}

/// Draw a 1-pixel rectangle outline into a buffer.
pub fn draw_rect_border(
    dest: &mut PixelBuffer<f32>,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: [f32; 4],
) {
    if w == 0 || h == 0 {
        return;
    }
    for ix in x..(x + w).min(dest.width as usize) {
        if y < dest.height as usize {
            dest.set_pixel(ix as u32, y as u32, color);
        }
        if y + h > 0 && y + h - 1 < dest.height as usize {
            dest.set_pixel(ix as u32, (y + h - 1) as u32, color);
        }
    }
    for iy in y..(y + h).min(dest.height as usize) {
        if x < dest.width as usize {
            dest.set_pixel(x as u32, iy as u32, color);
        }
        if x + w > 0 && x + w - 1 < dest.width as usize {
            dest.set_pixel((x + w - 1) as u32, iy as u32, color);
        }
    }
}

/// Generates a single contact sheet page containing up to `cols * rows` images.
pub fn generate_contact_sheet_page(
    image_paths: &[PathBuf],
    config: &ContactSheetConfig,
) -> Result<PixelBuffer<f32>, String> {
    if config.cols == 0 || config.rows == 0 {
        return Err("Columns and rows must be greater than zero".to_string());
    }

    let mut sheet = PixelBuffer::<f32>::new(config.width as u32, config.height as u32);
    let bg_color = config.background.color();
    let text_color = config.background.text_color();
    let border_color = config.background.border_color();

    // Fill background
    for y in 0..sheet.height {
        for x in 0..sheet.width {
            sheet.set_pixel(x, y, bg_color);
        }
    }

    let mut top_offset = config.margin;

    // Header Title
    if let Some(ref title) = config.title {
        if !title.trim().is_empty() {
            let title_scale = 3;
            let (tw, th) = text_dimensions(title, title_scale);
            let tx = if config.width > tw {
                (config.width - tw) / 2
            } else {
                config.margin
            };
            let ty = config.margin;
            render_text(&mut sheet, title, tx, ty, title_scale, text_color, false);

            // Subtle divider line under header
            let line_y = ty + th + 16;
            if line_y < config.height {
                for lx in config.margin..(config.width.saturating_sub(config.margin)) {
                    sheet.set_pixel(lx as u32, line_y as u32, border_color);
                }
            }
            top_offset = line_y + 24;
        }
    }

    let label_height = if config.show_filenames { 28 } else { 0 };
    let avail_w = config.width.saturating_sub(config.margin * 2 + (config.cols - 1) * config.spacing);
    let avail_h = config.height.saturating_sub(top_offset + config.margin + (config.rows - 1) * config.spacing);

    let cell_w = avail_w / config.cols;
    let cell_h = avail_h / config.rows;

    if cell_w < 10 || cell_h < 10 {
        return Err("Sheet dimensions too small for grid settings".to_string());
    }

    let photo_box_h = cell_h.saturating_sub(label_height);

    let backend = NativeBackend::default();

    for (i, path) in image_paths.iter().take(config.cols * config.rows).enumerate() {
        let col = i % config.cols;
        let row = i / config.cols;

        let cell_x = config.margin + col * (cell_w + config.spacing);
        let cell_y = top_offset + row * (cell_h + config.spacing);

        // Load image
        let img_buf = match backend.decode_from_path(path) {
            Ok(buf) => buf,
            Err(e) => {
                eprintln!("Skipping image {:?}: {}", path, e);
                continue;
            }
        };

        // Aspect fit into photo_box
        let scale_w = cell_w as f32 / img_buf.width as f32;
        let scale_h = photo_box_h as f32 / img_buf.height as f32;
        let fit_scale = scale_w.min(scale_h).min(1.0); // Don't upscale tiny icons beyond 1.0

        let thumb_w = ((img_buf.width as f32 * fit_scale).round() as u32).max(1);
        let thumb_h = ((img_buf.height as f32 * fit_scale).round() as u32).max(1);

        let resized = resize(&img_buf, thumb_w, thumb_h);

        // Center inside photo_box
        let img_x = cell_x + (cell_w.saturating_sub(thumb_w as usize)) / 2;
        let img_y = cell_y + (photo_box_h.saturating_sub(thumb_h as usize)) / 2;

        blit_buffer(&mut sheet, &resized, img_x, img_y);
        draw_rect_border(&mut sheet, img_x, img_y, thumb_w as usize, thumb_h as usize, border_color);

        // Filename label
        if config.show_filenames {
            if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                let caption_scale = 1;
                let truncated = truncate_to_width(file_name, cell_w, caption_scale);
                let (tw, _) = text_dimensions(&truncated, caption_scale);
                let text_x = cell_x + (cell_w.saturating_sub(tw)) / 2;
                let text_y = cell_y + photo_box_h + 8;
                render_text(&mut sheet, &truncated, text_x, text_y, caption_scale, text_color, false);
            }
        }
    }

    Ok(sheet)
}

/// Generates and exports contact sheet files to the target destination.
/// Returns a list of generated file paths.
pub fn generate_and_save_contact_sheets(
    image_paths: &[PathBuf],
    config: &ContactSheetConfig,
    output_dir: &Path,
    base_name: &str,
) -> Result<Vec<PathBuf>, String> {
    if image_paths.is_empty() {
        return Err("No images provided for contact sheet".to_string());
    }

    let per_page = config.cols * config.rows;
    let total_pages = (image_paths.len() + per_page - 1) / per_page;
    let ext = config.format.extension();
    let backend = NativeBackend::default();
    let options = ExportOptions {
        format: config.format,
        quality: config.quality,
    };

    let mut generated_paths = Vec::new();

    for page_idx in 0..total_pages {
        let start = page_idx * per_page;
        let end = (start + per_page).min(image_paths.len());
        let page_paths = &image_paths[start..end];

        let sheet = generate_contact_sheet_page(page_paths, config)?;

        let filename = if total_pages == 1 {
            format!("{}.{}", base_name, ext)
        } else {
            format!("{}_{:02}.{}", base_name, page_idx + 1, ext)
        };
        let out_file = output_dir.join(filename);

        backend
            .encode_to_path(&sheet, &out_file, &options)
            .map_err(|e| format!("Failed to save contact sheet to {:?}: {}", out_file, e))?;

        generated_paths.push(out_file);
    }

    Ok(generated_paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contact_sheet_config_defaults() {
        let cfg = ContactSheetConfig::default();
        assert_eq!(cfg.cols, 4);
        assert_eq!(cfg.rows, 4);
        assert_eq!(cfg.width, 2400);
        assert_eq!(cfg.height, 1800);
    }

    #[test]
    fn test_sheet_page_rendering() {
        let temp_dir = std::env::temp_dir().join("nodapix_test_contact_sheet");
        let _ = std::fs::create_dir_all(&temp_dir);

        // Create 2 dummy images
        let p1 = temp_dir.join("img1.png");
        let p2 = temp_dir.join("img2.png");

        let backend = NativeBackend::default();
        let options = ExportOptions {
            format: ExportFormat::Png,
            quality: 90,
        };

        let mut b1 = PixelBuffer::<f32>::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                b1.set_pixel(x, y, [1.0, 0.0, 0.0, 1.0]); // red
            }
        }
        backend.encode_to_path(&b1, &p1, &options).unwrap();

        let mut b2 = PixelBuffer::<f32>::new(80, 120);
        for y in 0..120 {
            for x in 0..80 {
                b2.set_pixel(x, y, [0.0, 0.0, 1.0, 1.0]); // blue
            }
        }
        backend.encode_to_path(&b2, &p2, &options).unwrap();

        let config = ContactSheetConfig {
            cols: 2,
            rows: 2,
            width: 400,
            height: 300,
            background: SheetBackground::White,
            show_filenames: true,
            title: Some("Test Sheet".to_string()),
            margin: 20,
            spacing: 10,
            format: ExportFormat::Png,
            quality: 90,
        };

        let paths = vec![p1.clone(), p2.clone()];
        let sheet = generate_contact_sheet_page(&paths, &config).unwrap();
        assert_eq!(sheet.width, 400);
        assert_eq!(sheet.height, 300);

        // Verify that red and blue pixels exist in sheet
        let mut found_red = false;
        let mut found_blue = false;
        for y in 0..sheet.height {
            for x in 0..sheet.width {
                let px = sheet.get_pixel(x, y);
                if px[0] > 0.9 && px[1] < 0.1 && px[2] < 0.1 {
                    found_red = true;
                }
                if px[2] > 0.9 && px[0] < 0.1 && px[1] < 0.1 {
                    found_blue = true;
                }
            }
        }
        assert!(found_red, "Contact sheet must contain rendered red image");
        assert!(found_blue, "Contact sheet must contain rendered blue image");

        // Clean up
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
